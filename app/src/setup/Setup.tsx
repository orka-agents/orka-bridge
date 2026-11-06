import { useEffect, useRef, useState } from 'react';
import { selectLocal, setupStatus, type SetupStatus } from './state';

export default function Setup() {
  const [status, setStatus] = useState<SetupStatus>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(true);
  const [check, setCheck] = useState(0);
  const epoch = useRef(0);
  useEffect(() => {
    const request = ++epoch.current;
    void setupStatus().then(result => {
      if (request !== epoch.current) return;
      if (result.ok) { setStatus(result.status); setError(undefined); }
      else { setStatus(undefined); setError(result.message); }
      setBusy(false);
    });
    return () => { epoch.current++; };
  }, [check]);

  async function chooseLocal() {
    if (busy || !status || status.selected === 'localVm') return;
    const request = ++epoch.current;
    setBusy(true); setError(undefined);
    const result = await selectLocal();
    if (request !== epoch.current) return;
    if (result.ok) setStatus(result.status);
    else { setStatus(undefined); setError(result.message); }
    setBusy(false);
  }
  return (
    <div className="setup">
      <section className="card setup-choice" aria-labelledby="setup-title" aria-busy={busy}>
        <h2 id="setup-title">Choose where your agents run</h2>
        <p className="muted">This saves a choice only. Nothing is downloaded or started.</p>
        <fieldset disabled={busy || !status}>
          <legend>Runtime</legend>
          <label><input type="radio" name="runtime" checked={status?.selected === 'localVm'}
            onChange={() => { void chooseLocal(); }} />
            <span>Local VM<span className="muted choice-detail">Use a VM on this computer when integration is available.</span></span>
          </label>
          <label><input type="radio" name="runtime" disabled checked={false} readOnly />
            <span>Existing cluster<span className="muted choice-detail">Not available yet. No connection will be attempted.</span></span>
          </label>
        </fieldset>
        <p role="status">{busy ? 'Checking the setup choice…' : !status ? 'Setup choice unavailable.'
          : status.selected === 'localVm' ? 'Local VM selected.' : 'No runtime selected.'}</p>
        {status && <div className="setup-availability">
          <h3>Native setup available</h3>
          <p className="muted">VM integration is not included in this build.</p>
        </div>}
        {error && <div role="alert"><p>{error}</p>
          <button type="button" className="setup-button" disabled={busy}
            onClick={() => { setBusy(true); setError(undefined); setCheck(value => value + 1); }}>Check again</button>
        </div>}
      </section>
      <section className="card setup-model" aria-labelledby="model-title">
        <h2 id="model-title">Model connection locked</h2>
        <p className="muted">Model connections require supported restricted worker access. This build cannot send credentials to a VM.</p>
        <button type="button" className="setup-button" disabled>Connect model</button>
      </section>
    </div>
  );
}
