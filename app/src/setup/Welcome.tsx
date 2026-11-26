import { Button } from '../components/Button';

export const welcomePreference = 'bridge.welcomeSeen.v1';
export function hasSeenWelcome() {
  try { return localStorage.getItem(welcomePreference) === '1'; }
  catch { return false; }
}

// Someone who already runs Orka picks their cluster on the next screens, beside this computer.
export default function Welcome({ titleId, onStart, unavailable }: { titleId?: string; onStart: () => void; unavailable?: string }) {
  return <section className="welcome-screen">
    <div className="art-orca" aria-hidden="true" />
    <h1 id={titleId}><span>Your agents. Your models.</span> <span>Your keys. Your cluster.</span></h1>
    <p className="welcome-lede">Build and test an agent on this computer, then send the same agent to your team's Orka cluster.</p>
    <ol className="welcome-journey">
      <li><span className="journey-number" aria-hidden="true">1</span><b>Write it</b><span>What it should do and which tools it can use.</span></li>
      <li><span className="journey-number" aria-hidden="true">2</span><b>Test it</b><span>Run it and check each answer.</span></li>
      <li><span className="journey-number" aria-hidden="true">3</span><b>Share it</b><span>Send it to your team's cluster (preview).</span></li>
    </ol>
    <div className="welcome-actions">
      <Button kind="primary" size="large" type="button" onClick={onStart}>Get started</Button>
      <span className="muted">About 5 minutes, mostly a download</span>
      {unavailable && <p className="muted">{unavailable}</p>}
    </div>
  </section>;
}
