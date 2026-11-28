import { useEffect, useRef, useState } from 'react';
import { type ChatCall, type ChatCommand, type ChatPlace, type ChatSnapshot, chatCall, chatErrors, snapshotErrorMessage } from './chat-api';

// A submitted message waiting to appear as a turn. scrollTop and readerMovedAway belong to the caller's scrolling.
export type PendingTurn = { message: string; count: number; scrollTop?: number; uncertain?: boolean; readerMovedAway?: boolean };
// A message that was refused or ended before it became a turn. It stays on screen with why, until the next send or New.
export type FailedSend = { message: string; reason: string };
const notSent = 'The message wasn’t sent. Nothing was created or changed.';
export type TurnOptions = {
  // Called when an accepted or reconciled message should bring the reader back to the end.
  resumeFollow?: () => void;
  // Read when a message is submitted, so the caller can tell later whether the reader moved.
  scrollTop?: () => number | undefined;
  // The native command set to use; Try a task's by default. The Navigator has its own.
  call?: ChatCall;
  // The selected place: a connected cluster, or this computer when absent. Every command names it, and
  // switching shows that place's own conversation.
  place?: ChatPlace;
  // How the native errors read for this caller; Try a task's by default.
  errors?: Record<keyof typeof chatErrors, string>;
};

// Polls one agent's native turn state and reconciles commands against it. A lost reply is never treated as a
// rejection: a submitted message stays pending until a turn for it, or its busy→idle settlement, is observed.
export function useTurn(documentId: string, blocked: string | undefined, options: TurnOptions = {}) {
  const [snapshot, setSnapshot] = useState<ChatSnapshot>();
  const latest = useRef<ChatSnapshot | undefined>(undefined);
  const [draft, setDraftState] = useState(''); const draftRef = useRef('');
  const [error, setError] = useState<{ message: string; uncertainSend?: boolean }>(); const [readError, setReadError] = useState<string>();
  const [command, setCommand] = useState<ChatCommand>();
  const mutation = useRef(false); const epoch = useRef(0); const alive = useRef(true);
  const pending = useRef<PendingTurn | undefined>(undefined);
  const [awaitingTurn, setAwaitingTurn] = useState(false);
  const [failed, setFailed] = useState<FailedSend>();
  const hooks = useRef(options); hooks.current = options;
  const placeKey = options.place ? `${options.place.targetId}/${options.place.providerId}` : '';
  const setDraft = (value: string) => { draftRef.current = value; setDraftState(value); };
  const receive = (value: ChatSnapshot) => {
    const turns = value.versions.flatMap(version => version.turns);
    const submitted = pending.current;
    if (submitted && turns.slice(submitted.count).some(turn => turn.message === submitted.message)) {
      if (submitted.uncertain) hooks.current.resumeFollow?.();
      if (draftRef.current === submitted.message) setDraft('');
      pending.current = undefined; setAwaitingTurn(false);
    } else if (submitted) {
      if (value.busy) {
        if (submitted.uncertain) hooks.current.resumeFollow?.();
        submitted.uncertain = false;
      }
      // A lost IPC reply is not a rejection. An idle snapshot alone may precede
      // admission; retain acknowledgement until a turn or its busy→idle is seen.
      if (!value.busy && (value.state === 'contextUnavailable' || !submitted.uncertain && value.state === 'ready')) {
        pending.current = undefined; setAwaitingTurn(false);
        // It ended before it became a turn: keep it on screen with why.
        setFailed({ message: submitted.message, reason: snapshotErrorMessage(value, hooks.current.errors) ?? notSent });
      }
    }
    if (submitted && !pending.current) setError(current => current?.uncertainSend ? undefined : current);
    latest.current = value; setSnapshot(value);
  };
  const receiveRef = useRef(receive); receiveRef.current = receive;
  useEffect(() => {
    alive.current = true; let mounted = true; let timer: ReturnType<typeof setTimeout>;
    const observe = async () => {
      if (!mounted) return;
      if (!mutation.current) {
        const request = epoch.current;
        const result = await (hooks.current.call ?? chatCall)('agent_chat_status', documentId, undefined, hooks.current.place);
        if (mounted && request === epoch.current) {
          if (result.ok) { receiveRef.current(result.snapshot); setReadError(undefined); } else setReadError(result.message);
        }
      }
      if (mounted) timer = setTimeout(() => { void observe(); }, 500);
    };
    void observe();
    return () => { mounted = false; alive.current = false; epoch.current++; clearTimeout(timer); };
  }, [documentId]);
  // Another place is another conversation: forget what this one showed, and drop any reply still on its way.
  const shownPlace = useRef(placeKey);
  useEffect(() => {
    if (shownPlace.current === placeKey) return;
    shownPlace.current = placeKey; epoch.current++; mutation.current = false;
    pending.current = undefined; latest.current = undefined;
    setSnapshot(undefined); setAwaitingTurn(false); setFailed(undefined); setError(undefined); setReadError(undefined); setCommand(undefined);
  }, [placeKey]);
  // Resolves true when the command was accepted, false when it was refused or uncertain, and undefined when it
  // was not dispatched or its reply arrived too late to apply.
  const act = async (action: ChatCommand, message?: string): Promise<boolean | undefined> => {
    const current = latest.current;
    if (mutation.current || !current) return;
    // The parent's block covers every conversation reset too; Stop/Reconnect stay available for recovery.
    if (action === 'agent_chat_new' && blocked) return;
    const text = message ?? draftRef.current;
    if (action === 'agent_chat_send' && (blocked || !current.canSend || pending.current || !text.trim())) return;
    mutation.current = true; const request = ++epoch.current; setCommand(action); setError(undefined);
    if (action === 'agent_chat_send' || action === 'agent_chat_new') setFailed(undefined);
    if (action === 'agent_chat_send') {
      pending.current = { message: text, count: current.versions.reduce((n, version) => n + version.turns.length, 0), scrollTop: hooks.current.scrollTop?.() };
      setAwaitingTurn(true);
    }
    const operation = action === 'agent_chat_reconnect' && pending.current?.uncertain ? 'agent_chat_status' : action;
    const result = await (hooks.current.call ?? chatCall)(operation, documentId, pending.current?.message, hooks.current.place);
    if (!alive.current || request !== epoch.current) return;
    if (result.ok) {
      if (action === 'agent_chat_send') hooks.current.resumeFollow?.();
      receiveRef.current(result.snapshot);
    } else {
      if (action === 'agent_chat_send') {
        if (result.uncertain && pending.current) pending.current.uncertain = true;
        else {
          pending.current = undefined; setAwaitingTurn(false);
          setFailed({ message: text, reason: result.message });
        }
      }
      setError({ message: result.message, uncertainSend: action === 'agent_chat_send' && result.uncertain });
    }
    mutation.current = false; setCommand(undefined);
    return result.ok;
  };
  return { snapshot, draft, setDraft, error, readError, command, awaitingTurn, pending, failed, act };
}
