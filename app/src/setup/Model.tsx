import { useEffect, useRef, useState } from 'react';
import { connectModel, removeModel } from './modelState';

export type ModelState = 'idle' | 'connecting' | 'removing' | 'connected' | 'failed';
export default function Model({ enabled, paused = false, onState, onSettled }: {
  enabled: boolean; paused?: boolean; onState?: (state: ModelState) => void; onSettled?: () => void;
}) {
  const [busy, setBusy] = useState<'connect' | 'remove'>();
  const [removed, setRemoved] = useState(false);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string>();
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
    setBusy(action); setError(undefined); setRemoved(false);
    if (action === 'remove') setReady(false);
    const result = await (action === 'connect' ? connectModel() : removeModel());
    if (request !== epoch.current) return;
    active.current = false; setBusy(undefined);
    if (result.ok) { setReady(action === 'connect'); setRemoved(action === 'remove'); }
    else setError(result.message);
    onSettled?.();
  }
  return <section className="card setup-model" aria-labelledby="model-title" aria-busy={!!busy}>
    <h2 id="model-title">{enabled ? 'Connect a model' : 'Model connection locked'}</h2>
    {!enabled ? <p className="muted">Start the local VM before connecting a model. No credentials are sent.</p>
      : <p className="muted">Use your existing GitHub CLI sign-in for Copilot in this VM. Bridge stores it in the native credential store and delivers it privately to the guest.</p>}
    <div className="workspace-step-actions">
      <button type="button" className="workspace-primary" disabled={!enabled || paused || !!busy || ready} onClick={() => { void perform('connect'); }}>
        {!enabled ? 'Connect model' : busy === 'connect' ? 'Connecting model…' : ready ? 'Model connected' : 'Use GitHub sign-in'}
      </button>
      <button type="button" className="workspace-link" disabled={!enabled || paused || !!busy} onClick={() => { void perform('remove'); }}>
        {busy === 'remove' ? 'Removing sign-in…' : 'Remove sign-in'}
      </button>
    </div>
    {busy === 'connect' && <p role="status">Connecting with Bridge’s saved sign-in, or reading your GitHub CLI sign-in if none is saved…</p>}
    {busy === 'remove' && <p role="status">Stopping the guest model and removing Bridge’s saved sign-in…</p>}
    {removed && <p role="status">Bridge no longer has a saved sign-in. GitHub CLI stays signed in.</p>}
    {ready && <><p role="status">Model connected: copilot / gpt-5.6-terra.</p>
      <p className="muted">The guest acknowledged Connect. This does not prove inference.</p></>}
    {error && <p role="alert">{error}</p>}
  </section>;
}
