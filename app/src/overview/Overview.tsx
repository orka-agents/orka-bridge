import { useEffect, useRef, useState } from 'react';
import { Button } from '../components/Button';
import { type LiveAgents, type Observation, type Places, type RunAgent, readAgents, readFiles, readPlaces, type SavedFiles, unavailableAgents } from './api';
import { agentGroups, balance, buildCards, clusterGroups, type CompleteAgents, type Inputs, isEmpty, summaryLabel } from './model';
import { Board, GroupSection, useCompact } from './places';
import { type AgentFocus, sendAgentCheck } from '../clusters/send-api';
import './overview.css';

export type OverviewProps = {
  onNewAgent?: () => void; active: boolean; runBusy: boolean; activeDocumentId?: string; workspaceKey: string;
  onOpenDocument: (id: string) => void; onChatDocument?: (id: string) => void; onRunDocument?: (id: string) => void;
  onOpenExample: () => void; onChooseWorkspace: () => void; onCounts?: (label: string) => void;
  /** Newest request wins. target is absent when native has no recorded agent for that Send. */
  focus?: { id: number; sendId: string; target?: AgentFocus };
};
const focusNotes = { present: 'Not shown in Overview yet.', gone: 'The original agent is no longer available. Your saved result is unchanged.', cannotCheck: 'Cannot check this agent. Your saved result is unchanged.' };
const complete = (v?: LiveAgents): v is CompleteAgents => v?.state === 'complete' && v.installation !== null && v.observedAt !== null;
export default function Overview({ onNewAgent, active, runBusy, activeDocumentId, workspaceKey, onOpenDocument, onChatDocument, onRunDocument, onOpenExample, onChooseWorkspace, onCounts, focus }: OverviewProps) {
  // Last good reads are kept so a busy or failed refresh never blanks the page; each is flagged when it is not fresh.
  const [files, setFiles] = useState<SavedFiles>();
  const [filesStale, setFilesStale] = useState(false);
  const finals = useRef(new Map<string, Observation>());
  const knownRunAgents = useRef(new Map<string, RunAgent>());
  const [live, setLive] = useState<LiveAgents>();
  const [last, setLast] = useState<CompleteAgents>();
  const [pending, setPending] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [byAgent, setByAgent] = useState(false);
  const [places, setPlaces] = useState<Places>();
  const [placeError, setPlaceError] = useState(false);
  const compact = useCompact();
  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    void readPlaces().then(value => {
      if (cancelled) return;
      setPlaceError(!value);
      setPlaces(previous => value ?? (previous && { ...previous, places: previous.places.map(p => ({ ...p, stale: true })) }));
    });
    return () => { cancelled = true; };
  }, [active, refresh]);
  // See in Overview: refresh only the recorded destination, then focus the exact card (place key + UID, never a name).
  const focusEpoch = useRef(0);
  const toFocus = useRef<string | undefined>(undefined);
  const [focusTick, setFocusTick] = useState(0);
  const [focusNote, setFocusNote] = useState<string>();
  useEffect(() => {
    if (!active || !focus) return;
    const request = ++focusEpoch.current;
    setByAgent(false); setFocusNote(undefined); toFocus.current = undefined;
    const { target } = focus;
    void (async () => {
      const value = target ? await readPlaces(target.key) : undefined;
      if (request !== focusEpoch.current) return;
      const seen = value?.places.find(p => p.key === target?.key);
      if (value && seen) setPlaces(previous => previous ? { ...previous, historyState: value.historyState, unobserved: value.unobserved, places: previous.places.some(p => p.key === seen.key) ? previous.places.map(p => p.key === seen.key ? seen : p) : [...previous.places, seen] } : value);
      if (target && seen?.agents.some(a => a.uid === target.uid && a.namespace === target.namespace)) { toFocus.current = `${target.key}/${target.uid}`; setFocusTick(v => v + 1); return; }
      const check = target ? await sendAgentCheck(focus.sendId) : 'cannotCheck';
      if (request === focusEpoch.current) setFocusNote(focusNotes[check]);
    })();
    return () => { focusEpoch.current++; };
  }, [active, focus, refresh]);
  useEffect(() => {
    const key = toFocus.current; if (!active || !key) return;
    const card = [...document.querySelectorAll<HTMLElement>('[data-card-key]')].find(node => node.dataset.cardKey === key);
    if (card) { toFocus.current = undefined; card.focus(); card.scrollIntoView?.({ block: 'center' }); }
  }, [active, focusTick, places, byAgent]);
  const epoch = useRef(0);
  const knownWorkspace = useRef(workspaceKey);
  useEffect(() => {
    const request = ++epoch.current;
    if (knownWorkspace.current !== workspaceKey) {
      knownWorkspace.current = workspaceKey; setLive(undefined); setLast(undefined);
    }
    if (!active) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    setPending(runBusy);
    async function list(retried = false) {
      const value = await readAgents();
      if (request !== epoch.current) return;
      setLive(value);
      if (complete(value)) { setLast(value); setPending(false); }
      else if (value.state === 'busy' && value.runActive) setPending(true);
      else if (value.state === 'busy' && !retried) timer = setTimeout(() => { void list(true); }, 250);
    }
    void readFiles().then(value => {
      if (request !== epoch.current) return;
      if (value.state === 'complete') {
        value.files.forEach(f => { if (f.localResult && (f.localResult.state === 'complete' || f.localResult.state === 'unknown' && (!(runBusy || value.runActive) || f.documentId !== activeDocumentId))) finals.current.set(f.documentId, f.localResult); });
        // Cleanup may forget a removed Agent while the cached inventory still contains it.
        value.runAgents.forEach(a => knownRunAgents.current.set(JSON.stringify([a.installation, a.namespace, a.agentUid]), a));
        setFiles({ ...value, runAgents: [...knownRunAgents.current.values()] }); setFilesStale(false);
      } else {
        setFiles(previous => previous?.state === 'complete' ? previous : value); setFilesStale(true);
        // Keep the last coherent inventory; fresh Agents cannot be filtered against stale run ownership.
        setLive(unavailableAgents); return;
      }
      if (runBusy || value.runActive) { setPending(true); return; }
      void list();
    });
    return () => { epoch.current++; clearTimeout(timer); };
  }, [active, runBusy, activeDocumentId, workspaceKey, refresh]);
  const observed = complete(live) ? live : last;
  const inputs: Inputs = { files, filesStale, finals: finals.current, live, observed, pending, activeDocumentId, places, placeError, chat: !!onChatDocument, run: !!onRunDocument };
  const built = buildCards(inputs);
  const empty = isEmpty(inputs, built);
  const partial = filesStale || files?.historyState !== 'complete' || !complete(live) || pending || !places || placeError || places.places.some(p => p.stale || p.observedAt === null);
  const counts = !files ? 'Reading saved files…' : files.state === 'unavailable' ? 'Saved files unavailable' : summaryLabel(built.cards, partial);
  useEffect(() => { if (active) onCounts?.(counts); }, [active, counts, onCounts]);
  const clusters = clusterGroups(inputs, built);
  const groups = byAgent ? agentGroups(built.cards, built.saved) : clusters;
  const actions = { onOpen: onOpenDocument, onChat: onChatDocument, onRun: onRunDocument, onCheck: () => setRefresh(v => v + 1), onClusters: onChooseWorkspace };
  const trouble = { ...clusters[0], cards: [], count: undefined, notes: clusters[0].notes.filter(n => !n.text.startsWith('No agents')) };
  return <section className="overview" aria-label="Agents by cluster" hidden={!active}>
    <div className="page-heading"><h1 id={active ? 'page-title' : undefined}>Your agents</h1><div className="overview-actions">
      {onNewAgent && !empty && <Button kind="primary" onClick={onNewAgent}>New agent</Button>}
      {!empty && <div className="view-switch" role="group" aria-label="Group agents"><button type="button" aria-pressed={!byAgent} onClick={() => setByAgent(false)}>By cluster</button><button type="button" aria-pressed={byAgent} onClick={() => setByAgent(true)}>By agent</button></div>}
      <Button kind="quiet" onClick={actions.onCheck}>Refresh</Button></div></div>
    {focusNote && <div className="overview-note" role="status"><p>{focusNote}</p><Button kind="quiet" onClick={actions.onCheck}>Refresh</Button></div>}
    {placeError && <div className="overview-note" role="status"><p>Couldn't read clusters just now.{places && ' Showing what Bridge last saw.'}</p><Button kind="quiet" onClick={actions.onCheck}>Check again</Button></div>}
    {places?.historyState === 'unavailable' && <p role="alert" className="overview-note">Saved Send results are unavailable, so cluster agents show no test result.</p>}
    {byAgent && !empty && <div className="overview-stack">{clusters.filter(g => g.notes.some(n => !n.text.startsWith('No agents'))).map(g => <GroupSection key={g.key} group={{ ...g, cards: [], count: undefined, notes: g.notes.filter(n => !n.text.startsWith('No agents')) }} byAgent={false} actions={actions} />)}</div>}
    {empty ? <>
      {!!trouble.notes.length && <GroupSection group={trouble} byAgent={false} actions={actions} />}
      <div className="card overview-empty"><h2>No agents yet</h2><p>Make an agent, or start from web-reader, the example that answers from a web page. Agents you test or send show up here, one card per cluster.</p>
        <div className="overview-card-actions">{onNewAgent && <Button kind="primary" onClick={onNewAgent}>New agent</Button>}<Button onClick={onOpenExample}>Open web-reader</Button></div></div>
    </> : <Board stacks={compact ? [groups, []] : balance(groups)} byAgent={byAgent} actions={actions} />}
  </section>;
}
