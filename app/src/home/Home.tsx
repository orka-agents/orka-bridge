import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Button } from '../components/Button';
import { StatusIcon, StatusPill } from '../components/DesignSystem';
import type { ChatSnapshot, ChatTurn } from '../designer/chat-api';
import { type AgentEntry, listDocuments } from '../designer/documents-api';
import { useTurn } from '../designer/use-turn';
import AgentCard from './AgentCard';
import { type Card, type ExplainCard, parseReply, sourceLabel } from './cards';
import { type CardRecord, selectConversationPlace, updateConversation, useConversation } from './conversation-store';
import { NAVIGATOR_ID, type NavigatorResult, type ThinksWith, navigatorCall, navigatorErrors } from './navigator-api';
import Prose from './Prose';
import { unavailableCopy } from '../setup/availability';
import TaskCard, { runLine, type TaskPlace } from './TaskCard';
import './home.css';

// What stands between the person and the Navigator. It thinks with the model of the selected place: the workspace on
// this computer, or a connected cluster's chosen AI connection (`place`).
export type NavigatorGate = { kind: 'ready'; place?: TaskPlace } | { kind: 'waiting' } | { kind: 'unavailable' } | { kind: 'noModel' } | { kind: 'unreachable'; name?: string } | { kind: 'noConnection'; name: string };
export type HomePage = 'Overview' | 'Designer' | 'Clusters';
type Props = {
  active: boolean; revision: number; gate: NavigatorGate;
  onOpenPage: (page: HomePage) => void; onOpenAgent: (documentId: string) => void; onAgentCreated: () => void;
  /** Absent where this computer can't run agents (a build without the local cluster), so it is never offered. */
  onThisComputer?: () => void; blockedFor?: (documentId: string) => string | undefined;
  /** Opens the list of clusters, rather than the selected one. */
  onManageClusters?: () => void;
  /** A cluster connected in this run that the Navigator can switch to, when this computer can't run it. */
  switchTo?: { name: string; choose: () => void };
};

const starters: { label: string; message: string }[] = [
  { label: 'Create a task', message: 'I want to give one of my agents a task.' },
  { label: 'Watch a repo', message: 'Review new pull requests on my repo automatically.' },
  { label: 'Write a charter', message: 'I want agents on the team cluster to ask before they change anything.' },
  { label: 'Set up an agent', message: 'Help me set up a new agent.' },
  { label: 'Explain Orka to me', message: 'I’m new to Orka. What is a Task, and how is it different from an agent?' },
];
const vendors: Record<Exclude<ThinksWith['kind'], 'cluster'>, string> = { copilot: 'GitHub Copilot', openai: 'OpenAI', anthropic: 'Anthropic' };

export default function Home(props: Props) {
  const { gate } = props;
  // Each place has its own conversation: switching shows that place's, and never continues another's.
  const placeKey = gate.kind === 'ready' && gate.place ? `${gate.place.targetId}/${gate.place.providerId}` : '';
  useLayoutEffect(() => { selectConversationPlace(placeKey); }, [placeKey]);
  if (gate.kind !== 'ready') return <Gated {...props} gate={gate} />;
  return <Navigator key={placeKey} {...props} />;
}

function Hero({ lede = true }: { lede?: boolean }) {
  return <div className="home-hero">
    <div className="art-orca" aria-hidden="true" />
    <h1 id="page-title">What do you want done?</h1>
    {lede && <p className="muted">The Navigator explains Orka and sets things up with you. Nothing is created until you confirm a card.</p>}
  </div>;
}

// A disabled message box under a card that says why, and what to do.
function Gated({ gate, onOpenPage, onThisComputer, onManageClusters, switchTo }: Props & { gate: Exclude<NavigatorGate, { kind: 'ready' }> }) {
  const card = gate.kind === 'noModel' ? {
    title: 'Connect the AI first', line: 'no model connected',
    body: 'The Navigator thinks with the model your Orka is connected to. This computer has none yet. Overview, Designer and Clusters still work.',
    actions: <><Button type="button" kind="primary" onClick={() => onOpenPage('Clusters')}>Connect the AI</Button><Button type="button" kind="secondary" onClick={() => onOpenPage('Clusters')}>Switch to another Orka</Button></>,
  } : gate.kind === 'unavailable' ? {
    title: unavailableCopy.title, line: 'not available on this computer',
    body: `This build can't run agents on this computer, so the Navigator thinks with a cluster's AI connection. ${switchTo ? `It can think with ${switchTo.name}.` : 'Connect a cluster you already have in Clusters.'} Overview and Designer still work.`,
    actions: switchTo ? <><Button type="button" kind="primary" onClick={switchTo.choose}>Switch to {switchTo.name}</Button>{onManageClusters && <Button type="button" kind="secondary" onClick={onManageClusters}>Open Clusters</Button>}</>
      : <Button type="button" kind="primary" onClick={onManageClusters ?? (() => onOpenPage('Clusters'))}>Open Clusters</Button>,
  } : gate.kind === 'waiting' ? {
    title: 'Waiting for the workspace to start.', line: 'waiting for the workspace',
    body: 'The Navigator thinks with the model in the workspace on this computer, so it can answer once the workspace is ready. Overview, Designer and Clusters still work.',
    actions: <Button type="button" kind="secondary" onClick={() => onOpenPage('Clusters')}>Open Clusters</Button>,
  } : gate.kind === 'unreachable' ? {
    title: `Can’t reach ${gate.name ?? 'the selected cluster'}. Check your network or VPN.`, line: `can’t reach ${gate.name ?? 'the cluster'}`,
    body: `The Navigator thinks with the model on the Orka you have selected, so it can’t answer until ${gate.name ?? 'the cluster'} is reachable. Nothing on the cluster has changed. Your agent files are on this computer and still open in Designer.`,
    actions: <><Button type="button" kind="primary" onClick={() => onOpenPage('Clusters')}>Check again</Button>{onThisComputer && <Button type="button" kind="secondary" onClick={onThisComputer}>Switch to This computer</Button>}{onManageClusters && <Button type="button" kind="quiet" onClick={onManageClusters}>Open Clusters</Button>}</>,
  } : {
    title: `Choose an AI connection for ${gate.name}.`, line: 'no AI connection chosen',
    body: `The Navigator thinks with the AI connection you choose for ${gate.name}, so it can answer once one is chosen in Clusters. Nothing on ${gate.name} has changed.`,
    actions: <><Button type="button" kind="primary" onClick={() => onOpenPage('Clusters')}>Open Clusters</Button>{onThisComputer && <Button type="button" kind="secondary" onClick={onThisComputer}>Switch to This computer</Button>}</>,
  };
  return <section className="home home-start" aria-labelledby="page-title">
    <Hero lede={false} />
    <article className="card nav-gate" aria-labelledby="nav-gate-title">
      <header className="nav-gate-head"><StatusIcon state="needs-you" /><h2 id="nav-gate-title">{card.title}</h2><StatusPill state="needs-you">Needs you</StatusPill></header>
      <p>{card.body}</p>
      <div className="nav-actions">{card.actions}</div>
    </article>
    <section className="nav-composer nav-composer-off" aria-label="Chat" aria-disabled="true">
      <p className="nav-composer-placeholder">Ask anything, or describe a task</p>
      <div className="nav-composer-foot"><span className="nav-line">Navigator · {card.line}</span><Button type="button" kind="secondary" size="large" disabled>Send</Button></div>
    </section>
  </section>;
}

// This computer's workspace predates the Navigator. Nothing is sent to it, so the routes that work are listed instead.
function TooOld({ onOpenPage, switchTo }: Pick<Props, 'onOpenPage' | 'switchTo'>) {
  const reason = 'The workspace on this computer can’t run the Navigator yet.';
  return <section className="home home-start" aria-labelledby="page-title">
    <Hero lede={false} />
    <article className="card nav-gate" aria-labelledby="nav-gate-title">
      <header className="nav-gate-head"><StatusIcon state="needs-you" /><h2 id="nav-gate-title">The workspace on this computer is older than this Bridge.</h2><StatusPill state="needs-you">Needs you</StatusPill></header>
      <p>It can’t run the Navigator yet, so nothing was sent to it. {switchTo ? `The Navigator can think with ${switchTo.name} instead. ` : ''}Designer still works here: Try a task runs on this computer.</p>
      <div className="nav-actions">
        {switchTo && <Button type="button" kind="primary" onClick={switchTo.choose}>Switch to {switchTo.name}</Button>}
        <Button type="button" kind={switchTo ? 'secondary' : 'primary'} onClick={() => onOpenPage('Designer')}>Open Designer</Button>
      </div>
    </article>
    <section className="nav-composer nav-composer-off" aria-label="Chat" aria-disabled="true">
      <p className="nav-composer-placeholder">Ask anything, or describe a task</p>
      <div className="nav-composer-foot"><span className="nav-line">Navigator · runs on this computer · workspace too old</span><Button type="button" kind="secondary" size="large" disabled title={reason}>Send</Button></div>
    </section>
    <div className="nav-starters" role="group" aria-label="Or start with">
      <span className="muted">Or start with</span>
      {starters.map(starter => <Button key={starter.label} type="button" kind="secondary" disabled title={reason}>{starter.label}</Button>)}
    </div>
  </section>;
}

type View = { kind: 'thinking' } | { kind: 'answer'; reply: ReturnType<typeof parseReply> } | { kind: 'failed'; reason?: string } | { kind: 'stopped' } | { kind: 'held'; state: ChatSnapshot['state'] };
// How one Navigator turn reads, from the native turn and, for the newest turn, the conversation state.
function turnView(turn: ChatTurn, newest: boolean, snapshot: ChatSnapshot | undefined): View {
  if (turn.answer?.trim() && turn.phase !== 'Failed') return { kind: 'answer', reply: parseReply(turn.answer) };
  if (turn.terminationUnconfirmed) return { kind: 'stopped' };
  if (newest && snapshot) {
    if (snapshot.busy) return { kind: 'thinking' };
    if (['observationLost', 'stopUnconfirmed', 'contextUnavailable'].includes(snapshot.state)) return { kind: 'held', state: snapshot.state };
    if (snapshot.error && !['model', 'connection', 'interrupted', 'response'].includes(snapshot.error)) return { kind: 'failed', reason: navigatorErrors[snapshot.error] };
  }
  return { kind: 'failed' };
}
const heldCopy: Partial<Record<ChatSnapshot['state'], [string, string]>> = {
  observationLost: ['The connection was lost while the Navigator was answering.', 'Reconnect to see its answer. Nothing was created or changed.'],
  stopUnconfirmed: ['Stop wasn’t confirmed.', 'The answer may still arrive. Nothing was created or changed.'],
  contextUnavailable: ['This conversation could not be confirmed.', 'Start a new conversation. Nothing was created or changed.'],
};

function Navigator({ active, revision, gate, onOpenPage, onOpenAgent, onAgentCreated, blockedFor, switchTo }: Props) {
  const place = gate.kind === 'ready' ? gate.place : undefined;
  const fieldId = useId();
  const field = useRef<HTMLTextAreaElement>(null);
  const end = useRef<HTMLDivElement>(null);
  const composing = useRef(false);
  const [thinksWith, setThinksWith] = useState<ThinksWith | null>(null);
  const [tooOld, setTooOld] = useState(false);
  const call = useCallback(async (...args: Parameters<typeof navigatorCall>) => {
    const result: NavigatorResult = await navigatorCall(...args);
    if (result.ok && result.thinksWith !== undefined) {
      const next = result.thinksWith;
      setThinksWith(current => current?.kind === next?.kind && current?.model === next?.model ? current : next);
    }
    if (result.ok && result.workspaceTooOld !== undefined) setTooOld(result.workspaceTooOld);
    return result;
  }, []);
  const { snapshot, draft, setDraft, error, readError, command, awaitingTurn, pending, failed, act } = useTurn(NAVIGATOR_ID, undefined,
    { call, errors: navigatorErrors, place: place && { targetId: place.targetId, providerId: place.providerId } });
  const conversation = useConversation();
  // Saved agents, for the task card's choices. Unreadable means none to offer.
  const [agents, setAgents] = useState<AgentEntry[]>([]);
  useEffect(() => {
    if (!active) return;
    let live = true;
    void listDocuments().then(list => { if (live) setAgents((list ?? []).filter(agent => agent.available)); });
    return () => { live = false; };
  }, [active, revision]);
  // A replaced workspace forgets its conversations natively; start this one over rather than hide new turns.
  const versionCount = snapshot?.versions.length;
  useEffect(() => {
    if (versionCount !== undefined && versionCount < conversation.startVersion) updateConversation(() => ({ startVersion: 0, cards: {} }));
  }, [versionCount, conversation.startVersion]);
  const turns = useMemo(() => snapshot?.versions.slice(conversation.startVersion).flatMap(version => version.turns) ?? [], [snapshot, conversation.startVersion]);
  const thinking = !!snapshot?.busy || awaitingTurn;
  const empty = !turns.length && !awaitingTurn && !failed;
  const reason = command ? 'Wait for this action to finish.' : !snapshot ? 'Opening the Navigator…' : thinking ? undefined
    : snapshot.readOnly ? 'This conversation was started with another AI connection. Start a new conversation to continue.'
    : !snapshot.canSend && snapshot.state !== 'contextUnavailable' ? 'Reconnect or stop the last answer first.' : undefined;
  const startOver = async () => {
    if (thinking) return;
    const versions = snapshot?.versions.length ?? 0;
    if (await act('agent_chat_new') !== true) return;
    updateConversation(() => ({ startVersion: versions, cards: {} }));
  };
  // A starter is the same as typing its text and sending: it fills the field, so a failed send leaves it there.
  const send = async (message: string) => {
    if (!message.trim() || reason || thinking) return;
    setDraft(message);
    if (snapshot?.state === 'contextUnavailable') await startOver();
    await act('agent_chat_send', message);
  };
  // Follow the conversation as it grows.
  const count = turns.length; const last = turns.at(-1)?.answer;
  // The message box sits in the flow after the turns, so the page's full scroll shows the newest turn above it.
  useEffect(() => {
    const page = end.current?.closest('.page-content');
    if (page) page.scrollTop = page.scrollHeight;
  }, [count, last, awaitingTurn]);
  // Grow the field with what is written, as Try a task does.
  useEffect(() => {
    const box = field.current;
    if (!box) return;
    box.style.height = 'auto';
    // Empty, the stylesheet sizes it (three lines, or one in a narrow window).
    box.style.height = draft ? `${box.scrollHeight}px` : '';
  }, [draft, empty]);
  // Only the newest proposal of each kind can act; an older one stays as it was, marked Not created.
  const latest = new Map<string, string>();
  turns.forEach(turn => {
    const card = turn.answer && turn.phase !== 'Failed' ? parseReply(turn.answer).card : undefined;
    if (card && card.kind !== 'explain') latest.set(card.kind, String(turn.startedAt));
  });
  // On a cluster the line says where it runs; the model is the one its last answer used.
  // The line always names the place whose conversation is on screen, then the model its last answer used.
  // When the line is too long for the box, the place's name shortens in the middle of the line; the model stays.
  const model = place ? thinksWith?.kind === 'cluster' ? thinksWith.model : undefined
    : thinksWith && thinksWith.kind !== 'cluster' ? `${vendors[thinksWith.kind]} · ${thinksWith.model}` : undefined;
  const line = <><span className="nav-line-place">runs on {place ? place.name : 'this computer'}</span>{model && <span className="nav-line-model"> · thinks with {model}</span>}</>;
  // A refused send shows on its own turn below; the field alert is for everything else.
  const failure = error && error.message !== failed?.reason ? error.message : readError;
  const composer = <form className={`nav-composer${empty ? ' nav-composer-start' : ''}`} aria-label="Chat" onSubmit={event => { event.preventDefault(); void send(draft); }}>
    <label className="sr-only" htmlFor={fieldId}>Message</label>
    <textarea id={fieldId} ref={field} rows={empty ? 3 : 1} placeholder="Ask anything, or describe a task" value={draft}
      onChange={event => setDraft(event.target.value)}
      onCompositionStart={() => { composing.current = true; }} onCompositionEnd={() => { composing.current = false; }}
      onKeyDown={event => {
        if (event.key === 'Enter' && !event.shiftKey && !composing.current && !event.nativeEvent.isComposing && event.keyCode !== 229) { event.preventDefault(); void send(draft); }
      }} />
    <div className="nav-composer-foot">
      <span className="nav-line" title={`Navigator · runs on ${place ? place.name : 'this computer'}${model ? ` · thinks with ${model}` : ''}`}><span className="nav-line-lead">Navigator · </span>{line}</span>
      {/* An empty conversation has nothing to start over, and the line gets the room. */}
      {!empty && <span className="nav-links">
        <Button type="button" kind="quiet" disabled={thinking || !!command} onClick={() => { void startOver(); }}>New conversation</Button>
      </span>}
      {thinking ? <Button type="button" kind="primary" size="large" disabled={command === 'agent_chat_stop' || snapshot?.state === 'requestingStop'} onClick={() => { void act('agent_chat_stop'); }}>Stop</Button>
        : <Button type="submit" kind="primary" size="large" disabled={!!reason || !draft.trim()} title={reason}>Send</Button>}
    </div>
    {failure && <div className="alert alert-failed"><p className="alert-title" role="alert">{failure}</p></div>}
  </form>;
  if (tooOld && !place && empty) return <TooOld onOpenPage={onOpenPage} switchTo={switchTo} />;
  if (empty) return <section className="home home-start" aria-labelledby="page-title">
    <Hero />
    {composer}
    <div className="nav-starters" role="group" aria-label="Or start with">
      <span className="muted">Or start with</span>
      {starters.map(starter => <Button key={starter.label} type="button" kind="secondary" disabled={!!reason || thinking} onClick={() => { void send(starter.message); }}>{starter.label}</Button>)}
    </div>
  </section>;
  return <section className="home home-conversation" aria-labelledby="page-title">
    <h1 id="page-title" className="sr-only">What do you want done?</h1>
    <ol className="nav-turns" aria-label="Conversation">
      {turns.map((turn, index) => <Turn key={`${turn.startedAt}-${index}`} turn={turn} newest={index === turns.length - 1 && !awaitingTurn} snapshot={snapshot}
        record={conversation.cards[String(turn.startedAt)]} actionable={kind => latest.get(kind) === String(turn.startedAt)}
        agents={agents} onAsk={message => { void send(message); }} onRetry={() => { void send(turn.message); }}
        onReconnect={() => { void act('agent_chat_reconnect'); }} onStartOver={() => { void startOver(); }}
        onOpenPage={onOpenPage} onOpenAgent={onOpenAgent} onAgentCreated={onAgentCreated} blockedFor={blockedFor} place={place} />)}
      {awaitingTurn && <li className="nav-turn"><p className="nav-user">{pending.current?.message}</p><Thinking /></li>}
      {failed && !awaitingTurn && <li className="nav-turn"><p className="nav-user">{failed.message}</p>
        <article className="card nav-card" aria-label="Not sent">
          <div className="alert alert-failed" role="alert"><p className="alert-title">Not sent.</p><p>{failed.reason}</p></div>
          <div className="nav-actions">
            {tooOld && !place ? <>{switchTo && <Button type="button" kind="primary" onClick={switchTo.choose}>Switch to {switchTo.name}</Button>}
              <Button type="button" kind={switchTo ? 'secondary' : 'primary'} onClick={() => onOpenPage('Designer')}>Open Designer</Button></>
              : <><Button type="button" kind="primary" disabled={!!reason} title={reason} onClick={() => { void send(failed.message); }}>Try again</Button>
                <Button type="button" kind="secondary" onClick={() => onOpenPage('Clusters')}>Open Clusters</Button></>}
          </div>
        </article>
      </li>}
    </ol>
    <div ref={end} />
    <div className="nav-composer-dock">{composer}</div>
  </section>;
}

function Thinking() {
  return <div className="nav-reply"><p className="section-label">Navigator</p><p className="nav-thinking" role="status"><StatusIcon state="running" />Thinking…</p></div>;
}

type TurnProps = {
  turn: ChatTurn; newest: boolean; snapshot?: ChatSnapshot; record?: CardRecord; actionable: (kind: string) => boolean; agents: AgentEntry[];
  onAsk: (message: string) => void; onRetry: () => void; onReconnect: () => void; onStartOver: () => void;
  onOpenPage: (page: HomePage) => void; onOpenAgent: (documentId: string) => void; onAgentCreated: () => void; blockedFor?: (documentId: string) => string | undefined;
  place?: TaskPlace;
};
function Turn({ turn, newest, snapshot, record, actionable, agents, onAsk, onRetry, onReconnect, onStartOver, onOpenPage, onOpenAgent, onAgentCreated, blockedFor, place }: TurnProps) {
  const view = turnView(turn, newest, snapshot);
  return <li className="nav-turn">
    <p className="nav-user">{turn.message}</p>
    {view.kind === 'thinking' && <Thinking />}
    {view.kind === 'stopped' && <div className="nav-reply"><p className="section-label">Navigator</p><p>Stopped. Nothing was created or changed.</p></div>}
    {view.kind === 'failed' && <article className="card nav-card" aria-label="The Navigator couldn’t answer">
      <div className="alert alert-failed" role="alert"><p className="alert-title">The Navigator couldn’t answer.</p><p>{view.reason ?? 'The model did not respond. Nothing was created or changed.'}</p></div>
      {newest && <div className="nav-actions"><Button type="button" kind="primary" onClick={onRetry}>Try again</Button><Button type="button" kind="secondary" onClick={() => onOpenPage('Clusters')}>Check the AI connection</Button></div>}
    </article>}
    {view.kind === 'held' && <article className="card nav-card" aria-label="The Navigator’s answer is not confirmed">
      <div className="alert alert-needs-you" role="alert"><p className="alert-title">{heldCopy[view.state]?.[0]}</p><p>{heldCopy[view.state]?.[1]}</p></div>
      <div className="nav-actions">{view.state === 'contextUnavailable' ? <Button type="button" kind="primary" onClick={onStartOver}>New conversation</Button>
        : <Button type="button" kind="primary" onClick={onReconnect}>Reconnect</Button>}</div>
    </article>}
    {view.kind === 'answer' && <Answer cardKey={String(turn.startedAt)} reply={view.reply} record={record ?? { status: 'proposed' }} actionable={actionable}
      agents={agents} onAsk={onAsk} onOpenPage={onOpenPage} onOpenAgent={onOpenAgent} onAgentCreated={onAgentCreated} blockedFor={blockedFor} place={place} />}
  </li>;
}

function Answer({ cardKey, reply, record, actionable, agents, onAsk, onOpenPage, onOpenAgent, onAgentCreated, blockedFor, place }: {
  cardKey: string; reply: ReturnType<typeof parseReply>; record: CardRecord; actionable: (kind: string) => boolean; agents: AgentEntry[];
  onAsk: (message: string) => void; onOpenPage: (page: HomePage) => void; onOpenAgent: (documentId: string) => void; onAgentCreated: () => void;
  blockedFor?: (documentId: string) => string | undefined; place?: TaskPlace;
}) {
  const card: Card | undefined = record.edited ?? reply.card;
  const task = card?.kind === 'task' && record.status === 'created' ? record.task : undefined;
  // Once a task exists, Bridge's own line about it replaces the proposal's words: the card now shows the Task.
  const outcome = task?.outcome;
  const words = task ? runLine(!outcome ? 'Running' : outcome.stopped || outcome.phase === 'Cancelled' ? 'Cancelled'
    : outcome.phase === 'Succeeded' ? 'Succeeded' : outcome.phase === 'Failed' ? 'Failed' : 'Unknown', task.where) : reply.prose;
  return <>
    <div className="nav-reply">
      <p className="section-label">Navigator</p>
      {words && <Prose text={words} />}
    </div>
    {card?.kind === 'task' && <TaskCard cardKey={cardKey} proposal={card} record={record} actionable={actionable('task')} agents={agents} blockedFor={blockedFor} onOpenAgent={onOpenAgent} onAsk={onAsk} place={place} />}
    {card?.kind === 'agent' && <AgentCard cardKey={cardKey} proposal={card} record={record} actionable={actionable('agent')} agents={agents}
      onCreated={onAgentCreated} onOpenAgent={onOpenAgent} />}
    {card?.kind === 'explain' && <Explain card={card} onAsk={onAsk} onOpenPage={onOpenPage} />}
    {/* The digest pages the answer drew on, named by the Navigator and checked against the bundled digest. */}
    {!task && reply.sources.length > 0 && <p className="nav-sources muted">From the docs: {reply.sources.map((source, index) => {
      const { site, page } = sourceLabel(source);
      return <span key={source}>{index ? ', ' : ''}{site} <code title={source}>{page}</code></span>;
    })}</p>}
  </>;
}

function Explain({ card, onAsk, onOpenPage }: { card: ExplainCard; onAsk: (message: string) => void; onOpenPage: (page: HomePage) => void }) {
  return <article className="card nav-card" aria-label={card.title}>
    <header className="nav-card-head"><h3 className="section-label">{card.title}</h3></header>
    <dl className="nav-rows">{card.rows.map(row => <div key={row.term}><dt>{row.term}</dt><dd>{row.text}</dd></div>)}</dl>
    {card.actions.length > 0 && <div className="nav-actions">{card.actions.map((action, index) =>
      <Button key={action.label} type="button" kind={index ? 'secondary' : 'primary'} onClick={() => 'ask' in action ? onAsk(action.ask) : onOpenPage(action.open)}>{action.label}</Button>)}</div>}
  </article>;
}
