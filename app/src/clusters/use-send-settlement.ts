import { useCallback, useEffect, useRef } from 'react';
import { SAMPLE_DOCUMENT_ID } from '../designer/agent-api';
import { sendCall, type SendStatus } from './send-api';

export type SendStart = (args: Record<string, unknown>) => Promise<SendStatus>;
type Attempt = { documentId: string; id: string; settled: boolean };

/** App owns admission observation; the editor may disappear before even the start reply. */
export function useSendSettlement(onSettled: () => void): SendStart {
  const requests = useRef(0);
  const accepted = useRef(0);
  const lifetime = useRef(0);
  const attempt = useRef<Attempt | undefined>(undefined);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const reading = useRef(false);
  useEffect(() => () => {
    lifetime.current++;
    clearTimeout(timer.current);
    attempt.current = undefined;
  }, []);
  const settle = useCallback((owned: Attempt) => {
    if (attempt.current !== owned || owned.settled) return;
    owned.settled = true;
    clearTimeout(timer.current);
    onSettled();
  }, [onSettled]);
  const observe = useCallback(async function observe() {
    const owned = attempt.current;
    if (!owned || owned.settled || reading.current) return;
    const generation = lifetime.current;
    reading.current = true;
    try {
      const result = await sendCall('send_status', { documentId: owned.documentId });
      if (generation !== lifetime.current || attempt.current !== owned || owned.settled) return;
      // A replacement view supersedes this attempt; invalidate, but never adopt its verdict.
      // No view (or an unreadable reply) is not evidence that the owned attempt finished.
      if (result.view && (result.view.id !== owned.id || result.view.state !== 'running')) settle(owned);
    } catch { /* Unknown read evidence retries observation only, never submission. */ }
    finally {
      reading.current = false;
      if (generation === lifetime.current && attempt.current && !attempt.current.settled) {
        timer.current = setTimeout(() => { void observe(); }, 500);
      }
    }
  }, [settle]);
  return useCallback(async args => {
    const request = ++requests.current;
    const generation = lifetime.current;
    const documentId = args.documentId ?? SAMPLE_DOCUMENT_ID;
    const result = await sendCall('send_start', args);
    const view = result.view;
    if (generation !== lifetime.current || !view || view.documentId !== documentId
      || result.busy && result.busy.documentId !== documentId || request < accepted.current) return result;
    // Only accepted ownership fences older replies. A later rejection/null/foreign busy reply cannot.
    accepted.current = request;
    let owned = attempt.current;
    if (!owned || owned.id !== view.id || owned.documentId !== view.documentId) {
      clearTimeout(timer.current);
      owned = { documentId: view.documentId, id: view.id, settled: false };
      attempt.current = owned;
      // An old in-flight read schedules the latest owner when it exits, so reads never overlap.
      if (view.state === 'running' && !reading.current) timer.current = setTimeout(() => { void observe(); }, 500);
    }
    if (view.state !== 'running') settle(owned);
    return result;
  }, [observe, settle]);
}
