import { useEffect, useRef, useState } from 'react';
import { agentCall, SAMPLE_DOCUMENT_ID } from '../designer/agent-api';
import { runCall, type RunSnapshot } from '../designer/run-api';
import type { CheckStepState } from './Steps';
import { loadExample } from './state';

const preference = 'bridge.firstTest.v1';
export function armFirstTest() {
  try { if (localStorage.getItem(preference) === null) localStorage.setItem(preference, 'pending'); } catch { /* No durable claim means no automatic side effect. */ }
}
export function retireFirstTest() {
  try { if (localStorage.getItem(preference) === 'pending') localStorage.setItem(preference, 'attempted'); } catch { /* An unreadable preference cannot admit a test. */ }
}
function pending() {
  try { return localStorage.getItem(preference) === 'pending'; } catch { return false; }
}
function claim() {
  if (!pending()) return false;
  try { localStorage.setItem(preference, 'attempted'); return true; } catch { return false; }
}
const text = (value: string) => value.replaceAll('\r\n', '\n');

export function useFirstTest(options: { ready: boolean; canOpen: boolean; sampleDirty: boolean; onOpen: () => void }): CheckStepState | undefined {
  const latest = useRef(options);
  useEffect(() => { latest.current = options; });
  const [check, setCheck] = useState<CheckStepState>();
  useEffect(() => {
    if (!options.ready || options.sampleDirty) { setCheck(value => value === 'running' ? 'unknown' : value); return; }
    if (!pending()) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const finish = (snapshot?: RunSnapshot) => {
      setCheck(snapshot?.state === 'complete' ? snapshot.report?.verdict ?? 'unknown' : 'unknown');
      if (latest.current.canOpen) latest.current.onOpen();
    };
    const observe = async (snapshot: RunSnapshot) => {
      if (cancelled) return;
      if (snapshot.state !== 'running' && snapshot.busy?.documentId !== SAMPLE_DOCUMENT_ID) { finish(snapshot); return; }
      setCheck('running');
      timer = setTimeout(() => { void runCall('agent_run_status').then(result => {
        if (cancelled) return;
        if (result.ok) void observe(result.snapshot); else finish();
      }); }, 500);
    };
    void Promise.all([loadExample(), agentCall('agent_open', undefined, SAMPLE_DOCUMENT_ID), runCall('agent_run_status')]).then(async ([bundled, saved, status]) => {
      // StrictMode's abandoned effect must not consume the durable one-shot.
      if (cancelled || !latest.current.ready || latest.current.sampleDirty || !claim()) return;
      if (!bundled.ok || text(saved.agent) !== text(bundled.pair.agent) || text(saved.tests) !== text(bundled.pair.tests) || !status.ok) { finish(); return; }
      if (status.snapshot.state !== 'idle') { void observe(status.snapshot); return; }
      if (!status.snapshot.canRun || status.snapshot.busy) { finish(); return; }
      // Claim is persisted before submission: an interrupted/unknown attempt is never replayed.
      const result = await runCall('agent_run');
      if (cancelled) return;
      if (result.ok) void observe(result.snapshot); else finish();
    }).catch(() => { if (!cancelled && claim()) finish(); });
    return () => { cancelled = true; clearTimeout(timer); };
  }, [options.ready, options.sampleDirty]);
  return check;
}
