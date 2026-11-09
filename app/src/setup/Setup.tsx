import { useEffect, useRef, useState } from 'react';
import Model, { type ModelState } from './Model';
import { type LocalVmState, type SetupStatus, selectLocal, setupStatus } from './state';

const pending = (state: LocalVmState) => ['preparing', 'installing', 'starting', 'closing'].includes(state);
const runtimeCopy: Record<LocalVmState, string> = {
  notIntegrated: 'VM integration is not included in this build.',
  idle: 'Select Local VM to install and start the development image.',
  preparing: 'Verifying the local image and runtime…',
  installing: 'Installing private VM files…',
  starting: 'Starting Orka and waiting for the guest connection…',
  ready: 'Orka is running on this computer.',
  closing: 'Stopping the local VM…',
  closed: 'The local VM has stopped.',
  failed: 'The local VM could not be started or its shutdown could not be confirmed. Existing files were retained.',
};
export default function Setup({ onOpenExample, onReady, onStatus, onModelState, active = true }: {
  onOpenExample: () => void; onReady: () => void; active?: boolean;
  onStatus?: (status: SetupStatus | undefined) => void;
  onModelState?: (state: ModelState) => void;
}) {
  const [status, setStatus] = useState<SetupStatus>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(true);
  const [check, setCheck] = useState(0);
  const epoch = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => {
    const request = ++epoch.current;
    void setupStatus().then(result => {
      if (request !== epoch.current) return;
      if (result.ok) { setStatus(result.status); setError(undefined); }
      else {
        // An unreadable observation pauses actions, not an already admitted connection.
        setError(result.message);
      }
      const waiting = result.ok && pending(result.status.localVm);
      setBusy(waiting);
      timer.current = setTimeout(() => setCheck(value => value + 1), waiting ? 250 : 1000);
      onStatus?.(result.ok ? result.status : undefined);
    });
    return () => { epoch.current++; clearTimeout(timer.current); };
  }, [check, onStatus]);
  useEffect(() => { if (status?.localVm === 'ready') onReady(); }, [status?.localVm, onReady]);

  async function chooseLocal() {
    if (busy || !status || status.selected === 'localVm') return;
    // Observation must not invalidate an explicit selection awaiting its reply.
    clearTimeout(timer.current);
    const request = ++epoch.current;
    setBusy(true); setError(undefined);
    const result = await selectLocal();
    if (request !== epoch.current) return;
    if (result.ok) setStatus(result.status);
    else { setStatus(undefined); setError(result.message); }
    onStatus?.(result.ok ? result.status : undefined);
    const waiting = result.ok && pending(result.status.localVm);
    setBusy(waiting);
    if (result.ok) setCheck(value => value + 1);
  }
  return (
    <>
    <h1 id={active ? 'page-title' : undefined}>Clusters</h1>
    <div className="setup">
      <section className="card setup-choice" aria-labelledby="setup-title" aria-busy={busy}>
        <h2 id="setup-title">Choose where your agents run</h2>
        <p className="muted">{status?.localVm === 'notIntegrated' ? 'This saves a choice only. Nothing is downloaded or started.'
          : 'Local VM uses the verified development image and starts Orka on this computer.'}</p>
        <fieldset disabled={busy || !status || !!error}>
          <legend>Runtime</legend>
          <label><input type="radio" name="runtime" checked={status?.selected === 'localVm'}
            onChange={() => { void chooseLocal(); }} />
            <span>Local VM<span className="muted choice-detail">Use a VM on this computer.</span></span>
          </label>
          <label><input type="radio" name="runtime" disabled checked={false} readOnly />
            <span>Existing cluster<span className="muted choice-detail">Not available yet. No connection will be attempted.</span></span>
          </label>
        </fieldset>
        <p role="status">{busy ? status ? runtimeCopy[status.localVm] : 'Checking the setup choice…' : !status ? 'Setup choice unavailable.'
          : status.selected === 'localVm' ? 'Local VM selected.' : 'No runtime selected.'}</p>
        {status && <div className="setup-availability">
          <h3>Native setup available</h3>
          <p className="muted">{runtimeCopy[status.localVm]}</p>
          {status.localVmError && <p role="alert">{status.localVmError}</p>}
          {status.localVm === 'ready' && <button type="button" className="setup-button" onClick={onOpenExample}>Open example</button>}
        </div>}
        {error && <div role="alert"><p>{error}</p>
          <button type="button" className="setup-button" disabled={busy}
            onClick={() => { setBusy(true); setError(undefined); setCheck(value => value + 1); }}>Check again</button>
        </div>}
      </section>
      <Model enabled={status?.localVm === 'ready' && status.model === 'available'} paused={!!error} onState={onModelState} onSettled={() => setCheck(value => value + 1)} />
    </div>
    </>
  );
}
