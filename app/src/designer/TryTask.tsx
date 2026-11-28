import './try-task.css';
import { type ReactNode, useEffect, useId, useRef } from 'react';
import { Button } from '../components/Button';
import { CardSection, type Status, StatusIcon, StatusPill } from '../components/DesignSystem';
import { Disclosure } from '../components/Disclosure';
import { relativeTime } from '../components/RelativeTime';
import { builtInTools } from './built-in-tools';
import { type ChatPlace, type ChatSnapshot, type ChatTurn, chatErrors, snapshotErrorMessage } from './chat-api';
import { modelLabel } from './model-label';
import { useTurn } from './use-turn';

// A task is one turn of the native conversation path, always in a new conversation, so nothing carries between tasks.
const states: Record<ChatSnapshot['state'], string> = {
  ready: 'Ready for a task.', preparing: 'Preparing your saved agent…', working: 'Your agent is working…',
  waitingForInput: 'Your agent requested input that Try a task does not support. You can request Stop.',
  waitingForSession: 'Waiting for the task to finish saving…',
  observationLost: 'The connection was lost. Reconnect to check what happened; do not run the task again.',
  requestingStop: 'Requesting stop…', stopRequested: 'Stop requested. Execution termination is unconfirmed.',
  stopUnconfirmed: 'Stop request unconfirmed. Execution may still be running.',
  contextUnavailable: 'The last task could not be confirmed. You can run a new task.',
};
const steps = ['Pending', 'Scheduled', 'Running', 'Finalizing'] as const;
const terminal = ['Succeeded', 'Failed', 'Cancelled'];
const busyStates: ChatSnapshot['state'][] = ['preparing', 'working', 'waitingForSession', 'requestingStop'];
function stepIndex(phase: string) {
  if (['', 'Pending', 'Queued'].includes(phase)) return 0;
  if (phase === 'Scheduled') return 1;
  if (['Running', 'WaitingForInput', 'InputRequired'].includes(phase)) return 2;
  if (phase === 'Finalizing') return 3;
  return terminal.includes(phase) ? steps.length : -1;
}
type Outcome = { state: Status; word: string; note?: string };
// How a task ended, in the shared status words. `live` is the native state while this is the current task.
function outcome(turn: ChatTurn, live?: ChatSnapshot['state'], lost?: string): Outcome {
  if (live === 'requestingStop') return { state: 'running', word: 'Stopping', note: states.requestingStop };
  if (live === 'stopUnconfirmed' || live && turn.stopUnconfirmed) return { state: 'stale', word: 'Result unknown', note: states.stopUnconfirmed };
  if (live === 'observationLost') return { state: 'stale', word: 'Result unknown', note: lost ?? states.observationLost };
  if (live === 'waitingForInput') return { state: 'needs-you', word: 'Needs you', note: states.waitingForInput };
  if (live && busyStates.includes(live) && !terminal.includes(turn.phase)) return { state: 'running', word: 'Running' };
  const stopped = turn.terminationUnconfirmed ? states.stopRequested : turn.stopUnconfirmed ? states.stopUnconfirmed
    : turn.finishedBeforeStop ? 'This task had already finished when Stop arrived, so nothing was stopped.' : undefined;
  if (turn.phase === 'Succeeded') return { state: 'passed', word: 'Succeeded', note: stopped };
  if (turn.phase === 'Failed') return { state: 'failed', word: 'Failed', note: stopped };
  if (turn.phase === 'Cancelled') return { state: 'stale', word: 'Cancelled', note: stopped ?? 'The cluster reported this task cancelled; worker termination is unconfirmed.' };
  if (live && busyStates.includes(live)) return { state: 'running', word: 'Running' };
  if (turn.terminationUnconfirmed) return { state: 'stale', word: 'Stop requested', note: states.stopRequested };
  return { state: 'stale', word: 'Result unknown', note: stopped ?? 'No result was confirmed for this task.' };
}
const toolStates: Record<string, [Status, string]> = {
  ToolCallStarted: ['running', 'Running'], ToolCallCompleted: ['passed', 'Done'], ToolCallFailed: ['failed', 'Failed'],
};
// One row per tool call, in the order the calls started, with each call's latest event.
function toolCalls(tools: ChatTurn['tools'], settled: boolean) {
  const calls = new Map<string, { name: string; phase: string }>();
  for (const tool of tools) calls.set(tool.callId, { name: tool.name, phase: tool.phase });
  return [...calls.values()].map(({ name, phase }) => {
    const known = toolStates[phase] ?? (/^(succeeded|completed)$/i.test(phase) ? ['passed', 'Done'] : /^failed$/i.test(phase) ? ['failed', 'Failed'] : /^(running|started)$/i.test(phase) ? ['running', 'Running'] : ['not-started', phase]);
    const [state, word] = settled && known[0] === 'running' ? ['stale', 'No result'] as [Status, string] : known;
    return { name, state, word };
  });
}
export function duration(ms: number) {
  const seconds = Math.max(0, Math.round(ms / 1000));
  if (seconds < 1) return 'under a second';
  if (seconds < 60) return `${seconds} s`;
  const minutes = Math.floor(seconds / 60);
  return seconds % 60 ? `${minutes} min ${seconds % 60} s` : `${minutes} min`;
}
function ToolRow({ tool }: { tool: { name: string; state: Status; word: string } }) {
  const label = Object.hasOwn(builtInTools, tool.name) ? builtInTools[tool.name] : undefined;
  return <li className="try-tool"><StatusIcon state={tool.state} /><span>{label && <>{label} </>}<code className="code-chip">{tool.name}</code></span><span className="muted">{tool.word}</span></li>;
}
function Result({ label, turn, live, lost, waiting, model, actions }: { label: string; turn: ChatTurn; live?: ChatSnapshot['state']; lost?: string; waiting?: string; model?: string; actions?: ReactNode }) {
  const end = outcome(turn, live, lost);
  const settled = end.state !== 'running' && end.state !== 'needs-you';
  const at = stepIndex(turn.phase);
  const finalState: Status = turn.phase === 'Succeeded' ? 'passed' : turn.phase === 'Failed' ? 'failed' : 'stale';
  const calls = toolCalls(turn.tools, settled);
  const took = turn.finishedAt !== null && turn.startedAt > 0 ? duration(turn.finishedAt - turn.startedAt) : undefined;
  return <article className="card try-result" aria-label={`${label}: ${end.word}`}>
    <header className="try-result-head">
      <h3 className="section-label">Task{turn.startedAt > 0 && <> · {relativeTime(turn.startedAt)}</>}</h3>
      <StatusPill state={end.state}>{end.word}</StatusPill>
    </header>
    <p className="try-result-task">{turn.message}</p>
    {at >= 0 && <ol className="try-phases" aria-label="Phases">
      {steps.map((step, index) => <li key={step}><StatusIcon state={index < at ? 'passed' : index === at ? settled ? 'stale' : 'running' : 'not-started'} />{step}</li>)}
      {at === steps.length && <li><StatusIcon state={finalState} />{turn.phase}</li>}
    </ol>}
    {at < 0 && turn.phase && <p className="muted">Last reported phase: {turn.phase}</p>}
    {end.note && <div className={`alert${end.state === 'failed' ? ' alert-failed' : end.state === 'needs-you' ? ' alert-needs-you' : ''}`}><p className="alert-title">{end.note}</p></div>}
    {turn.phase === 'Failed' && <div className="alert alert-failed"><p className="alert-title">This task failed.</p><p>Your agent file and test results are unchanged.</p></div>}
    {!!calls.length && <CardSection title={calls.length === 1 ? 'Used a tool' : 'Used tools'}>
      <ul className="try-tools" aria-label="Tools used">{calls.slice(-3).map((tool, index) => <ToolRow key={index} tool={tool} />)}</ul>
      {calls.length > 3 && <Disclosure label={`${calls.length - 3} more tool call${calls.length === 4 ? '' : 's'}`}>
        <ul className="try-tools">{calls.slice(0, -3).map((tool, index) => <ToolRow key={index} tool={tool} />)}</ul>
      </Disclosure>}
    </CardSection>}
    <CardSection title="Answered">
      {/* A one-word answer is a value (a checksum, an id): it reads in mono, like code. */}
      {turn.answer !== null ? <p className={/^\S+$/.test(turn.answer.trim()) ? 'try-answer-value' : 'reading'}>{turn.answer}</p>
        : waiting ? <p className="try-waiting"><StatusIcon state="running" />{waiting}</p>
        : <p className="muted">{turn.phase === 'Failed' ? 'The task failed without an answer.' : 'No answer was received.'}</p>}
    </CardSection>
    {(took || model) && <p className="try-result-meta muted">{took && <>Took {took}</>}{took && model && ' · '}{model && <>Thinks with <code>{modelLabel(model)}</code></>}</p>}
    {actions && <div className="try-result-actions">{actions}</div>}
  </article>;
}

// Errors that already say why a cluster can't take the task; the status line stays quiet beside them.
const clusterErrors: ChatSnapshot['error'][] = ['permission', 'sessionsUnsupported', 'clusterUnavailable', 'accessLost'];
// This computer's workspace didn't answer one request. If it stopped answering entirely, the error is workspaceUnavailable.
const localConnection = 'The workspace on this computer didn’t answer. Run the task again. If it keeps happening, quit Bridge and open it again to restart the workspace.';
export default function TryTask({ documentId, agentName, model, blocked, place, active = true }: {
  documentId: string; agentName: string; model?: string; blocked?: string; active?: boolean;
  // A connected cluster and its chosen AI connection; absent means this computer.
  place?: ChatPlace & { name: string };
}) {
  const where = place?.name ?? 'this computer';
  // Home and Designer can show the same agent at once, so element ids are per instance.
  const instance = useId(); const fieldId = `task-${documentId}-${instance}`; const hintId = `task-hint-${documentId}-${instance}`;
  const composer = useRef<HTMLTextAreaElement>(null); const composing = useRef(false);
  // The model each task was started with, by its place in this agent's list of tasks. Memory only.
  const models = useRef(new Map<number, string | undefined>());
  const { snapshot, draft, setDraft, error, readError, command, awaitingTurn, pending, failed, act } = useTurn(documentId, blocked,
    { place: place && { targetId: place.targetId, providerId: place.providerId } });
  useEffect(() => {
    if (!active) return;
    const field = composer.current;
    if (!field) return;
    const fit = () => {
      field.style.height = 'auto';
      if (draft) field.style.height = `${field.scrollHeight}px`;
    };
    fit();
    window.addEventListener('resize', fit);
    return () => window.removeEventListener('resize', fit);
  }, [draft, active]);
  const turns = snapshot?.versions.flatMap(version => version.turns) ?? [];
  const current = turns.at(-1);
  const earlier = turns.slice(0, -1).map((turn, index) => ({ turn, index })).reverse();
  const live = snapshot?.state;
  const held = !!snapshot && !snapshot.busy && !snapshot.canSend && snapshot.state !== 'contextUnavailable';
  // A conversation from another AI connection is only shown; running a task starts a new one first.
  const runnable = !!snapshot && !snapshot.busy && (snapshot.canSend || snapshot.readOnly || snapshot.state === 'contextUnavailable');
  const reason = blocked ?? (command ? 'Wait for this action to finish.' : awaitingTurn ? 'Wait for this task to be accepted.' : !snapshot ? 'Opening Try a task…'
    : held && ['observationLost', 'stopUnconfirmed'].includes(snapshot.state) ? 'Check the last task first: reconnect or stop it.'
    : !runnable ? 'Wait for the current task or test to finish.' : undefined);
  // When a command settles, the control used may be disabled or gone. Hand focus back to the task field, unless the
  // person moved it somewhere else while the command was in flight.
  const settle = async (work: () => Promise<unknown>) => {
    const before = document.activeElement;
    await work();
    const now = document.activeElement;
    if (!now || now === document.body || now === before) composer.current?.focus();
  };
  // What the field holds when Run is pressed: the DOM value can be ahead of the last rendered draft when keys
  // arrive faster than React renders, and the task sent must be exactly what the person sees.
  const submitted = () => {
    const value = composer.current?.value ?? draft;
    if (value !== draft) setDraft(value);
    return value;
  };
  const run = (message: string) => settle(async () => {
    if (reason || !message.trim()) return;
    const index = turns.length;
    // A new conversation first, so the task starts clean. A refusal (a held or busy task) stops here.
    if (await act('agent_chat_new') !== true) return;
    models.current.set(index, model);
    await act('agent_chat_send', message);
  });
  const status = pending.current?.uncertain ? 'Checking whether your task was accepted. Do not run it again.'
    : command === 'agent_chat_stop' ? states.requestingStop
    : snapshot?.state === 'working' && current && ['', 'Pending', 'Queued', 'Scheduled'].includes(current.phase) ? 'Waiting to start…'
    // The hint below the field already says why; one message, not two.
    : blocked && snapshot?.state === 'ready' ? undefined
    // Access that can no longer authenticate is not a lost connection: reconnecting the cluster shows the result.
    : snapshot?.state === 'observationLost' && snapshot.error === 'accessLost' ? chatErrors.accessLost
    : snapshot ? states[snapshot.state] : 'Opening Try a task…';
  const routine = !status || status === states.ready || status === states.contextUnavailable && (!snapshot?.error || clusterErrors.includes(snapshot.error));
  const canStop = !!snapshot && snapshot.state !== 'ready' && snapshot.state !== 'contextUnavailable';
  const canReconnect = !!snapshot && snapshot.state !== 'contextUnavailable' && !snapshot.busy && (!snapshot.canSend || !!pending.current?.uncertain);
  const snapshotError = snapshot?.state === 'ready' && snapshot.error === 'connection'
    ? place ? `${place.name} could not be reached. Check Clusters, then try running the task again.` : localConnection
    : snapshot ? snapshotErrorMessage(snapshot) : undefined;
  // The task that wasn't run stays in the field; this says why. One message, not two.
  const failure = error?.message ?? readError ?? snapshotError ?? failed?.reason;
  const confirming = !!current && (snapshot?.state === 'waitingForSession' || snapshot?.state === 'working' && terminal.includes(current.phase));
  const waiting = !current || current.answer !== null ? undefined
    : confirming ? snapshot?.state === 'waitingForSession' ? states.waitingForSession : 'Confirming your agent’s reply…'
    : live && ['preparing', 'working'].includes(live) && ['', 'Pending', 'Queued', 'Scheduled', 'Running', 'Finalizing'].includes(current.phase)
      ? ['Running', 'Finalizing'].includes(current.phase) ? 'Waiting for your agent’s reply…' : 'Waiting for your agent to start…'
    : undefined;
  const recovery = <>
    {canStop && <Button type="button" kind="secondary" disabled={!!command || snapshot.state === 'requestingStop'} onClick={() => { void settle(() => act('agent_chat_stop')); }}>Stop</Button>}
    {canReconnect && <Button type="button" kind="secondary" disabled={!!command} onClick={() => { void settle(() => act('agent_chat_reconnect')); }}>Reconnect</Button>}
  </>;
  // Without a turn for it yet, recovery belongs to the composer; once the task shows, to its result.
  const recoveryOnCard = !!current && !awaitingTurn && (canStop || canReconnect);
  return <section className="try-task" aria-label={`Try a task on ${where}`}>
    <form className="card card-strong try-composer" onSubmit={event => { event.preventDefault(); void run(submitted()); }}>
      <label className="try-composer-label" htmlFor={fieldId}>Give <span className="try-agent">{agentName}</span> one task</label>
      <textarea className="field" ref={composer} id={fieldId} rows={2} placeholder="Describe the task" value={draft} aria-describedby={hintId}
        onChange={event => setDraft(event.target.value)}
        onCompositionStart={() => { composing.current = true; }} onCompositionEnd={() => { composing.current = false; }}
        onKeyDown={event => {
          if (event.key === 'Enter' && !event.shiftKey && !composing.current && !event.nativeEvent.isComposing && event.keyCode !== 229) {
            event.preventDefault(); void run(submitted());
          }
        }} />
      <div className="try-composer-foot">
        <p className="muted" id={hintId}>{reason ?? `Runs once on ${where}. It does not change test results.`}<span className="sr-only"> Enter runs the task · Shift+Enter for a new line</span></p>
        <Button type="submit" kind="primary" size="large" disabled={!!reason || !draft.trim()} title={reason ?? (!draft.trim() ? 'Describe the task first.' : undefined)}>Run task</Button>
      </div>
      <p className={`try-status${routine ? ' sr-only' : ''}`} role="status" aria-live="polite" aria-atomic="true">{routine && !current ? undefined : status}</p>
      {failure && <div className="alert alert-failed"><p role="alert" className="alert-title">{failure}</p></div>}
      {place && snapshot?.readOnly && <p className="muted">This task ran with another AI connection on {place.name}. The next task starts fresh with the one chosen now.</p>}
      {!recoveryOnCard && (canStop || canReconnect) && <div className="try-recovery">{recovery}</div>}
      {place && snapshot?.keptOnCluster && <p className="muted">Earlier tasks stay on {place.name}: your access there doesn’t allow Bridge to delete them.</p>}
    </form>
    {current ? <Result label="Task result" turn={current} live={live} lost={snapshot?.error === 'accessLost' ? chatErrors.accessLost : undefined} waiting={waiting} model={models.current.get(turns.length - 1)}
      actions={recoveryOnCard ? recovery : <Button type="button" kind="secondary" disabled={!!reason} title={reason} onClick={() => { void run(current.message); }}>Run again</Button>} />
      : <p className="empty-state try-empty">No tasks run yet. The result shows here: what it used, what it answered, and how the task ended.</p>}
    {!!earlier.length && <Disclosure className="try-earlier" label={`Earlier tasks: ${earlier.length}`}>
      <div className="try-earlier-list">{earlier.map(({ turn, index }) => <Result key={index} label="Earlier task" turn={turn} model={models.current.get(index)} />)}</div>
    </Disclosure>}
    {/* What Try a task keeps and what Stop can promise, from the native conversation path. */}
    <Disclosure className="try-about" label="About tasks here">
      <div className="try-about-copy muted">
        <p>Earlier tasks are kept only while Bridge is open.</p>
        <p>Each task starts fresh and doesn’t remember earlier ones.</p>
        <p>Stop asks the task to stop. A tool that already started may finish, and what it did isn’t undone.</p>
        {place ? <>
          <p>On {place.name}, Bridge deletes a task’s agent and record when you run the next one. Quitting Bridge leaves the last task there.</p>
          <p>If a task can’t be stopped, it ends at its two-minute limit.</p>
        </> : <p>If a task can’t be stopped, restart the workspace.</p>}
      </div>
    </Disclosure>
  </section>;
}
