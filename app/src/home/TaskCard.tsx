import { useEffect, useId, useRef, useState } from 'react';
import { Button } from '../components/Button';
import { type Status, StatusIcon, StatusPill } from '../components/DesignSystem';
import { agentCall } from '../designer/agent-api';
import { readForm } from '../designer/agent-form';
import { builtInTools } from '../designer/built-in-tools';
import type { ChatTurn } from '../designer/chat-api';
import type { AgentEntry } from '../designer/documents-api';
import { testDefinitions } from '../designer/test-definitions';
import { duration } from '../designer/TryTask';
import { useTurn } from '../designer/use-turn';
import type { TaskProposal } from './cards';
import { type CardRecord, type TaskOutcome, type TaskRun, updateCard } from './conversation-store';

// A proposed task runs exactly as Try a task does: one new conversation on the chosen saved agent, on this
// computer. The card then follows that Task: Pending, Scheduled, Running, Finalizing, then how it ended.
const steps = ['Pending', 'Scheduled', 'Running', 'Finalizing'] as const;
function stepIndex(phase: string) {
  if (['', 'Pending', 'Queued'].includes(phase)) return 0;
  if (phase === 'Scheduled') return 1;
  if (['Running', 'WaitingForInput', 'InputRequired'].includes(phase)) return 2;
  if (phase === 'Finalizing') return 3;
  return -1;
}
type Ending = 'Running' | 'Succeeded' | 'Failed' | 'Cancelled' | 'Unknown';
function ending(outcome: TaskOutcome | undefined, live: boolean): Ending {
  if (!outcome || live) return 'Running';
  if (outcome.stopped || outcome.phase === 'Cancelled') return 'Cancelled';
  if (outcome.phase === 'Succeeded') return 'Succeeded';
  if (outcome.phase === 'Failed') return 'Failed';
  return 'Unknown';
}
const pill: Record<Ending, [Status, string]> = {
  Running: ['running', 'Running'], Succeeded: ['passed', 'Succeeded'], Failed: ['failed', 'Failed'], Cancelled: ['stale', 'Cancelled'], Unknown: ['stale', 'Result unknown'],
};
// The words Bridge says above the card once the task exists. They describe the Task, not the Navigator's view of it.
export const runLine: Record<Ending, string> = {
  Running: 'Created. It is running on this computer.',
  Succeeded: 'Created. It ran on this computer and finished.',
  Failed: 'It failed. Your agent file and its test results are unchanged.',
  Cancelled: 'Stopped.',
  Unknown: 'Created. Bridge could not confirm how it ended.',
};

function Phases({ outcome, end }: { outcome: TaskOutcome; end: Ending }) {
  const at = end === 'Running' ? Math.max(stepIndex(outcome.phase), 0) : end === 'Succeeded' ? steps.length : outcome.reached;
  const final = end === 'Running' || end === 'Unknown' ? 'Succeeded' : end;
  return <ol className="nav-phases" aria-label="Phases">
    {steps.map((step, index) => <li key={step} className={index < at ? 'nav-step-done' : index > at ? 'nav-step-waiting' : undefined}>
      <StatusIcon state={index < at ? 'passed' : index === at ? 'running' : 'not-started'} />{step}
    </li>)}
    <li className={end === 'Succeeded' ? undefined : 'nav-step-waiting'}><StatusIcon state={end === 'Succeeded' ? 'passed' : 'not-started'} />{final}</li>
  </ol>;
}

function toolNames(tools: ChatTurn['tools']) { return [...new Set(tools.map(tool => tool.name))]; }
function UsedTools({ names }: { names: string[] }) {
  return <>{names.map((name, index) => <span key={name}>{index ? ', ' : 'Used '}{Object.hasOwn(builtInTools, name) && <>{builtInTools[name]} </>}<code>{name}</code></span>)}</>;
}

// The saved agent's description and first test title, as the proposal shows them.
function useAgentFacts(documentId: string | undefined) {
  const [facts, setFacts] = useState<{ id: string; description?: string; test?: string }>();
  useEffect(() => {
    if (!documentId) return;
    let live = true;
    void agentCall('agent_open', undefined, documentId).then(pair => {
      if (live) setFacts({ id: documentId, description: readForm(pair.agent).description || undefined, test: testDefinitions(pair.tests)?.find(test => test.title)?.title });
    }, () => { if (live) setFacts({ id: documentId }); });
    return () => { live = false; };
  }, [documentId]);
  return facts?.id === documentId ? facts : undefined;
}

type Props = {
  cardKey: string; proposal: TaskProposal; record: CardRecord; actionable: boolean; agents: AgentEntry[];
  // Designer's reason an agent can't take a task now, such as unsaved edits; the task runs the saved file.
  blockedFor?: (documentId: string) => string | undefined; onOpenAgent: (documentId: string) => void; onAsk: (message: string) => void;
};
export default function TaskCard(props: Props) {
  const { record } = props;
  if (record.status === 'created' && record.task) {
    return record.task.outcome ? <TaskRunCard {...props} run={record.task} /> : <LiveTaskRun {...props} run={record.task} />;
  }
  if (record.status === 'editing' && props.actionable) return <EditTask {...props} />;
  return <ProposedTask {...props} />;
}

function chosenAgent(agents: AgentEntry[], name: string) { return agents.find(agent => agent.name === name); }

function ProposedTask({ cardKey, proposal, record, actionable, agents, blockedFor, onAsk }: Props) {
  const agent = chosenAgent(agents, proposal.agent);
  const facts = useAgentFacts(agent?.id);
  const open = actionable && record.status === 'proposed';
  const reason = !agent ? `${proposal.agent} is not one of your saved agents. Choose one with Edit.` : blockedFor?.(agent.id);
  const [error, setError] = useState<string>();
  return <article className="card nav-card" aria-label="Proposed task">
    <header className="nav-card-head"><h3 className="section-label">Proposed task</h3><StatusPill state="not-started">Not created</StatusPill></header>
    <dl className="nav-rows">
      <div><dt>Agent</dt><dd>{agent ? <><code>{agent.name}</code>{facts?.description && <> · {facts.description}</>}</> : <><code>{proposal.agent}</code> · not found</>}</dd></div>
      <div><dt>Task</dt><dd className="nav-task-text">{proposal.task}</dd></div>
      <div><dt>Runs on</dt><dd>This computer</dd></div>
      {facts?.test && <div><dt>Checked by</dt><dd>the agent’s test: {facts.test}</dd></div>}
    </dl>
    {error && <div className="alert alert-failed"><p className="alert-title" role="alert">{error}</p></div>}
    {open && <div className="nav-actions">
      <CreateTask documentId={agent?.id} message={proposal.task} reason={reason} cardKey={cardKey} agentName={agent?.name} onError={setError} />
      <Button type="button" kind="secondary" onClick={() => updateCard(cardKey, current => ({ ...current, status: 'editing' }))}>Edit</Button>
      <Button type="button" kind="quiet" onClick={() => { updateCard(cardKey, current => ({ ...current, status: 'dismissed' })); onAsk('Just answer here, without creating a task.'); }}>Just answer here</Button>
    </div>}
    {open && reason && <p className="muted nav-reason">{reason}</p>}
  </article>;
}

type Kind = 'primary' | 'secondary' | 'quiet';
// Create task starts the Task the way Try a task does: a fresh conversation on that agent, then one message.
function CreateTask({ documentId, message, reason, cardKey, agentName, onError, label = 'Create task', kind = 'primary' }: {
  documentId?: string; message: string; reason?: string; cardKey: string; agentName?: string; onError: (message?: string) => void;
  label?: string; kind?: Kind;
}) {
  if (!documentId || !agentName) return <Button type="button" kind={kind} disabled title={reason}>{label}</Button>;
  return <CreateTaskOn documentId={documentId} message={message} reason={reason} cardKey={cardKey} agentName={agentName} onError={onError} label={label} kind={kind} />;
}
function CreateTaskOn({ documentId, message, reason, cardKey, agentName, onError, label, kind }: {
  documentId: string; message: string; reason?: string; cardKey: string; agentName: string; onError: (message?: string) => void; label: string; kind: Kind;
}) {
  const { snapshot, act, command, error } = useTurn(documentId, reason);
  const busy = !snapshot || snapshot.busy || !!command || !snapshot.canSend && snapshot.state !== 'contextUnavailable';
  const why = reason ?? (busy ? 'Wait for this agent’s current task or test to finish.' : undefined);
  useEffect(() => { onError(error?.message); }, [error, onError]);
  const create = async () => {
    if (why) return;
    const requestedAt = Date.now();
    if (await act('agent_chat_new') !== true) return;
    if (await act('agent_chat_send', message) !== true) return;
    updateCard(cardKey, current => ({ ...current, status: 'created', task: { documentId, agent: agentName, message, requestedAt } }));
  };
  return <Button type="button" kind={kind} disabled={!!why} aria-busy={command ? 'true' : undefined} title={why} onClick={() => { void create(); }}>{label}</Button>;
}

function EditTask({ cardKey, proposal, agents }: Props) {
  const id = useId();
  const [agentName, setAgentName] = useState(chosenAgent(agents, proposal.agent)?.name ?? '');
  const [task, setTask] = useState(proposal.task);
  const facts = useAgentFacts(chosenAgent(agents, agentName)?.id);
  const valid = !!chosenAgent(agents, agentName) && task.trim().length > 0 && task.length <= 4000;
  return <article className="card card-editing nav-card" aria-label="Proposed task">
    <header className="nav-card-head"><h3 className="section-label">Proposed task</h3><StatusPill state="running">Editing</StatusPill></header>
    <div className="nav-edit-grid">
      <label className="nav-field" htmlFor={`${id}-agent`}><span>Agent</span>
        <select id={`${id}-agent`} className="field nav-mono" value={agentName} onChange={event => setAgentName(event.target.value)}>
          {!chosenAgent(agents, agentName) && <option value="">Choose an agent</option>}
          {agents.map(agent => <option key={agent.id} value={agent.name}>{agent.name}</option>)}
        </select>
      </label>
      <label className="nav-field" htmlFor={`${id}-place`}><span>Runs on</span>
        <select id={`${id}-place`} className="field" defaultValue="thisComputer" title="Tasks from the Navigator run on this computer for now."><option value="thisComputer">This computer</option></select>
      </label>
    </div>
    <label className="nav-field" htmlFor={`${id}-task`}><span>Task</span>
      <textarea id={`${id}-task`} className="field" rows={3} value={task} onChange={event => setTask(event.target.value)} />
    </label>
    {facts?.test && <p className="muted nav-reason">Checked by the agent’s test: {facts.test}</p>}
    <div className="nav-actions">
      <Button type="button" kind="primary" disabled={!valid} onClick={() => updateCard(cardKey, current => ({ ...current, status: 'proposed', edited: { kind: 'task', agent: agentName, task } }))}>Save changes</Button>
      <Button type="button" kind="secondary" onClick={() => updateCard(cardKey, current => ({ ...current, status: 'proposed' }))}>Discard changes</Button>
    </div>
  </article>;
}

// Follows the agent's conversation until the Task this card created settles, then keeps how it ended.
function LiveTaskRun(props: Props & { run: TaskRun }) {
  const { run, cardKey, blockedFor } = props;
  const { snapshot, act, command, error } = useTurn(run.documentId, blockedFor?.(run.documentId));
  const reached = useRef(0);
  const turns = snapshot?.versions.flatMap(version => version.turns) ?? [];
  const turn = [...turns].reverse().find(item => item.message === run.message && item.startedAt >= run.requestedAt - 5000);
  const last = turns.at(-1);
  const live = !!snapshot && !!turn && turn === last && (snapshot.busy || snapshot.state === 'working');
  // A Task that used a tool or answered was running, even when a poll never caught that phase.
  if (turn) reached.current = Math.max(reached.current, stepIndex(turn.phase), turn.tools.length || turn.answer ? 2 : 0);
  const outcome: TaskOutcome | undefined = turn && { phase: turn.phase, answer: turn.answer, tools: toolNames(turn.tools), startedAt: turn.startedAt, finishedAt: turn.finishedAt, stopped: turn.terminationUnconfirmed, reached: Math.max(reached.current, 0) };
  const settled = !!turn && !!snapshot && !live && !snapshot.busy && (['Succeeded', 'Failed', 'Cancelled'].includes(turn.phase) || turn.terminationUnconfirmed || turn.answer !== null);
  useEffect(() => {
    if (settled && outcome) updateCard(cardKey, current => current.task && !current.task.outcome ? { ...current, task: { ...current.task, outcome } } : current);
  });
  const held = !!snapshot && !snapshot.busy && ['observationLost', 'stopUnconfirmed'].includes(snapshot.state);
  return <TaskRunCard {...props} run={{ ...run, outcome: outcome ?? { phase: '', answer: null, tools: [], startedAt: run.requestedAt, finishedAt: null, stopped: false, reached: 0 } }} live={!settled}
    stopping={command === 'agent_chat_stop' || snapshot?.state === 'requestingStop'}
    onStop={() => { void act('agent_chat_stop'); }} onReconnect={held ? () => { void act('agent_chat_reconnect'); } : undefined} error={error?.message} />;
}

function TaskRunCard({ cardKey, run, live = false, stopping, onStop, onReconnect, error, blockedFor, onOpenAgent }: Props & {
  run: TaskRun & { outcome?: TaskOutcome }; live?: boolean; stopping?: boolean; onStop?: () => void; onReconnect?: () => void; error?: string;
}) {
  const outcome = run.outcome!;
  const blocked = blockedFor?.(run.documentId);
  const end = ending(outcome, live);
  const [state, word] = pill[end];
  const [failure, setFailure] = useState<string>();
  const took = outcome.startedAt > 0 ? duration((outcome.finishedAt ?? Date.now()) - outcome.startedAt) : undefined;
  const plain = !!outcome.answer && !/\s/.test(outcome.answer.trim()) && outcome.answer.trim().length <= 200;
  return <article className="card nav-card" aria-label={`Task ${run.agent}: ${word}`}>
    <header className="nav-card-head"><h3 className="section-label">Task <b className="nav-agent-name">{run.agent}</b></h3><StatusPill state={state}>{word}</StatusPill></header>
    <Phases outcome={outcome} end={end} />
    {end === 'Running' && <div className="nav-card-section">
      <h4 className="section-label">So far</h4>
      {outcome.tools.length ? <p><UsedTools names={outcome.tools} /></p> : <p className="muted">No tools used yet.</p>}
      <p className="muted">{stopping ? 'Stopping…' : `Running for ${took ?? 'a moment'} on this computer`}</p>
    </div>}
    {end === 'Succeeded' && <div className="nav-card-section">
      <h4 className="section-label">Result</h4>
      {outcome.answer ? <p className={plain ? 'nav-result-value' : 'reading nav-result-text'}>{outcome.answer}</p> : <p className="muted">No answer was received.</p>}
      {!!outcome.tools.length && <p className="muted"><UsedTools names={outcome.tools} /></p>}
    </div>}
    {end === 'Failed' && <div className="alert alert-failed" role="alert"><p className="alert-title">The task failed while running.</p><p>{outcome.answer?.trim() || 'The agent stopped without answering.'}</p></div>}
    {end === 'Cancelled' && <p className="nav-card-section">You stopped this task{took ? ` after ${took}` : ''}. Orka was asked to stop it; a step already taken is not undone.</p>}
    {end === 'Unknown' && <p className="nav-card-section">Bridge could not confirm how this task ended. Open the agent’s Try a task view to check.</p>}
    {(error || failure) && <div className="alert alert-failed"><p className="alert-title" role="alert">{error ?? failure}</p></div>}
    <div className="nav-actions">
      {end === 'Running' && <>
        <Button type="button" kind="secondary" disabled={stopping || !onStop} onClick={onStop}>Stop task</Button>
        {onReconnect && <Button type="button" kind="secondary" onClick={onReconnect}>Reconnect</Button>}
        <Button type="button" kind="quiet" onClick={() => onOpenAgent(run.documentId)}>See in Designer</Button>
      </>}
      {end === 'Succeeded' && <>
        <Button type="button" kind="secondary" onClick={() => onOpenAgent(run.documentId)}>See in Designer</Button>
        <Rerun cardKey={cardKey} run={run} blocked={blocked} kind="quiet" label="Run again" onError={setFailure} />
      </>}
      {end === 'Failed' && <>
        <Rerun cardKey={cardKey} run={run} blocked={blocked} kind="primary" label="Try again" onError={setFailure} />
        <Button type="button" kind="secondary" onClick={() => onOpenAgent(run.documentId)}>Open {run.agent}</Button>
      </>}
      {(end === 'Cancelled' || end === 'Unknown') && <>
        <Rerun cardKey={cardKey} run={run} blocked={blocked} kind="secondary" label="Run again" onError={setFailure} />
        <Button type="button" kind="quiet" onClick={() => onOpenAgent(run.documentId)}>See in Designer</Button>
      </>}
    </div>
  </article>;
}

// Running again replaces this card's Task with a new one on the same agent and message.
function Rerun({ cardKey, run, blocked, kind, label, onError }: { cardKey: string; run: TaskRun; blocked?: string; kind: Kind; label: string; onError: (message?: string) => void }) {
  return <CreateTask documentId={run.documentId} message={run.message} reason={blocked} cardKey={cardKey} agentName={run.agent} onError={onError} label={label} kind={kind} />;
}
