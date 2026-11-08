import { useEffect, useRef, useState } from 'react';
import { selectLocal, setupStatus, type LocalVmState, type SetupStatus } from './state';
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
export default function Setup({ onOpenExample, onReady }: { onOpenExample: () => void; onReady: () => void }) {
  const [status, setStatus] = useState<SetupStatus>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(true);
  const [check, setCheck] = useState(0);
  const epoch = useRef(0);
  useEffect(() => {
    const request = ++epoch.current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    void setupStatus().then(result => {
      if (request !== epoch.current) return;
      if (result.ok) { setStatus(result.status); setError(undefined); }
      else { setStatus(undefined); setError(result.message); }
      const waiting = result.ok && pending(result.status.localVm);
      setBusy(waiting);
      if (waiting) timer = setTimeout(() => setCheck(value => value + 1), 250);
    });
    return () => { epoch.current++; clearTimeout(timer); };
  }, [check]);
  useEffect(() => { if (status?.localVm === 'ready') onReady(); }, [status?.localVm, onReady]);

  async function chooseLocal() {
    if (busy || !status || status.selected === 'localVm') return;
    const request = ++epoch.current;
    setBusy(true); setError(undefined);
    const result = await selectLocal();
    if (request !== epoch.current) return;
    if (result.ok) setStatus(result.status);
    else { setStatus(undefined); setError(result.message); }
    const waiting = result.ok && pending(result.status.localVm);
    setBusy(waiting);
    if (waiting) setCheck(value => value + 1);
  }
  return (
    <div className="setup">
      <section className="card setup-choice" aria-labelledby="setup-title" aria-busy={busy}>
        <h2 id="setup-title">Choose where your agents run</h2>
        <p className="muted">{status?.localVm === 'notIntegrated' ? 'This saves a choice only. Nothing is downloaded or started.'
          : 'Local VM uses the verified development image and starts Orka on this computer.'}</p>
        <fieldset disabled={busy || !status}>
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
          {status.localVm === 'ready' && <button type="button" className="setup-button" onClick={onOpenExample}>Open example</button>}
        </div>}
        {error && <div role="alert"><p>{error}</p>
          <button type="button" className="setup-button" disabled={busy}
            onClick={() => { setBusy(true); setError(undefined); setCheck(value => value + 1); }}>Check again</button>
        </div>}
      </section>
      <section className="card setup-model" aria-labelledby="model-title">
        <h2 id="model-title">Model connection locked</h2>
        <p className="muted">Model connection isn't enabled on this screen. No credentials are sent.</p>
        <button type="button" className="setup-button" disabled>Connect model</button>
      </section>
    </div>
  );
}
