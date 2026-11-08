import { useEffect, useState } from 'react';
import { loadExample, type ExamplePair } from '../setup/state';
import Run from './Run';
import type { RunSnapshot } from './run-api';
export default function Example({ onPair, onRun }: { onPair?: (pair: ExamplePair | undefined) => void; onRun?: (snapshot: RunSnapshot) => void }) {
  const [pair, setPair] = useState<ExamplePair>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    let active = true;
    void loadExample().then(result => {
      if (!active) return;
      if (result.ok) { setPair(result.pair); onPair?.(result.pair); } else { setError(result.message); onPair?.(undefined); }
    });
    return () => { active = false; };
  }, [onPair]);
  return <section className="card setup" aria-labelledby="example-title">
    <h2 id="example-title">Read-only example</h2>
    <p className="muted">These are the original web-reader agent and test files. Opening them doesn't run a test or connect a model.</p>
    {error ? <p role="alert">{error}</p> : !pair ? <p role="status">Opening the example…</p> : <>
      <Run onSnapshot={onRun} />
      <h3>web-reader.agent.md</h3>
      <pre className="example-source" tabIndex={0} aria-label="Agent file, read-only">{pair.agent}</pre>
      <h3>web-reader.agent.test.md</h3>
      <pre className="example-source" tabIndex={0} aria-label="Test file, read-only">{pair.tests}</pre>
    </>}
  </section>;
}
