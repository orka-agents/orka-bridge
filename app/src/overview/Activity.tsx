import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { Button } from '../components/Button';
import { StatusPill } from '../components/DesignSystem';
import { Disclosure } from '../components/Disclosure';
import { RelativeTime } from '../components/RelativeTime';
import { type Activity, type Filter, latest, merge, phaseView, type PlaceActivity, readActivity, readToolCalls, type Row, type ToolCalls } from './activity-api';
import type { Places } from './api';
import { placeLabels, settle, withAccess } from './model';

const filters: { id: Filter; label: string }[] = [{ id: 'all', label: 'All' }, { id: 'failed', label: 'Failed' }, { id: 'active', label: 'In progress' }];
const emptyFilter: Record<Filter, string> = { all: 'No runs yet on connected clusters.', failed: 'No failed runs among the latest runs.', active: 'Nothing is running or waiting.' };

/** An expanded run's tool calls, read again whenever its place is read again. */
function ToolCallList({ placeKey, task, revision }: { placeKey: string; task: string; revision: number | null }) {
  const [calls, setCalls] = useState<ToolCalls>();
  useEffect(() => {
    let cancelled = false;
    void readToolCalls(placeKey, task).then(value => { if (!cancelled) setCalls(value); });
    return () => { cancelled = true; };
  }, [placeKey, task, revision]);
  if (!calls) return <p className="overview-activity-meta" role="status">Reading tool calls…</p>;
  if (calls.state === 'denied') return <p className="overview-activity-meta" role="status">This account can't read this run's events.</p>;
  if (calls.state !== 'complete' && calls.state !== 'partial') return <p className="overview-activity-meta" role="status">Couldn't read tool calls just now.</p>;
  if (!calls.calls.length && calls.state === 'complete') return <p className="overview-activity-meta">No tool calls recorded.</p>;
  return <>
    {calls.calls.length > 0 && <ol className="overview-activity-tools">{calls.calls.map((c, i) => <li key={i}><code>{c.tool}</code>{c.failed && ' · failed'} · <RelativeTime value={c.at} /></li>)}</ol>}
    {calls.state === 'partial' && <p className="overview-activity-meta">Some of this run's tool calls aren't shown.</p>}
  </>;
}

function Entry({ row, place, agentShown, onShowAgent }: { row: Row; place: string; agentShown: boolean; onShowAgent: (key: string) => void }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const view = phaseView[row.phase];
  const subject = row.agent ?? row.task;
  return <li className="overview-activity-entry" aria-label={`${subject} on ${place}, ${view.pill}`}>
    <div className="overview-activity-row">
      <div className="overview-activity-name">
        <h3 className="overview-agent-title">{subject}</h3>
        <p className="overview-activity-meta">on <code>{place}</code> · run <code>{row.task}</code>{row.test && ' · Bridge test'}</p>
      </div>
      <StatusPill state={view.status}>{view.pill}</StatusPill>
      <span className="overview-time"><RelativeTime value={latest(row)} /></span>
      {agentShown && row.agentUid && <Button size="small" aria-label={`Show ${subject} on ${place}`} onClick={() => onShowAgent(`${row.placeKey}/${row.agentUid}`)}>Show agent</Button>}
      <Disclosure label="Tool calls" open={open} onOpenChange={setOpen} controls={id}>{null}</Disclosure>
    </div>
    {row.message && <p className="overview-activity-message">{row.message}</p>}
    {open && <div id={id} className="overview-activity-details"><ToolCallList placeKey={row.placeKey} task={row.task} revision={row.readAt} /></div>}
  </li>;
}

/** Recent runs across connected clusters, read only. The local VM cannot list Tasks yet. */
/** `revision` changes once per Overview visit or check, after its first places read lands; each new revision reads
 * activity once, so Send polling of places does not rescan Tasks. `onShowAgent` receives the agent's card key
 * (`place/uid`) and moves focus to that card. `onCheck` rereads Overview's places, which starts a new revision. */
export function RecentActivity({ active, places, revision, onShowAgent, onCheck }: { active: boolean; places?: Places; revision: number; onShowAgent: (key: string) => void; onCheck: () => void }) {
  const [activity, setActivity] = useState<Activity>();
  const [failed, setFailed] = useState(false);
  const [filter, setFilter] = useState<Filter>('all');
  const request = useRef(0);
  // Read after each places read, so only places this session just confirmed are listed. One read at a time:
  // places that change during a read queue one more read rather than starting another native round.
  const reading = useRef(false);
  const queued = useRef(false);
  const start = useCallback(function next() {
    if (reading.current) { queued.current = true; return; }
    reading.current = true;
    const current = ++request.current;
    void readActivity().then(value => {
      reading.current = false;
      // A superseded response is dropped, but a read queued behind it still runs. Unmounting clears the queue.
      if (current === request.current) {
        setFailed(!value);
        if (value) setActivity(previous => ({
          // An unreadable place keeps what Bridge last read on the same connection, so a refresh never blanks the
          // list, and a refusal stands until a Task read succeeds. A denied or revoked place, or a new connection,
          // drops its runs: access was refused or replaced.
          places: value.places.map(p => {
            const prior = previous?.places.find(q => q.key === p.key);
            const keep = (p.state === 'unavailable' || p.state === 'disconnected') && p.connection !== null && prior?.connection === p.connection;
            if (keep && prior?.state === 'denied') return { ...prior, state: 'denied' as const };
            return keep && prior?.entries.length ? { ...p, entries: prior.entries, observedAt: prior.observedAt } : p;
          }),
        }));
      }
      if (queued.current) { queued.current = false; next(); }
    });
  }, []);
  // Read once per revision: a revisit with cached places waits for the visit's own places read.
  const readFor = useRef<number>(undefined);
  useEffect(() => {
    if (active && places && revision !== readFor.current) { readFor.current = revision; start(); }
    // Overview stays mounted while hidden: drop a queued read, and the next visit reads afresh.
    if (!active) queued.current = false;
  }, [active, places, revision, start]);
  useEffect(() => () => { request.current++; queued.current = false; readFor.current = undefined; }, []);
  // A refusal, revocation or new connection from any places read stands until this section's own read succeeds.
  useEffect(() => {
    if (places) setActivity(previous => { if (!previous) return previous; const next = settle(previous.places, places.places); return next === previous.places ? previous : { ...previous, places: next }; });
  }, [places]);
  const labels = placeLabels(places?.places ?? []);
  const known = new Set(places?.places.flatMap(p => p.agents.map(a => `${p.key}/${a.uid}`)) ?? []);
  const read = withAccess(activity?.places ?? [], places?.places ?? []).filter(p => labels.has(p.key));
  const connected = read.filter(p => p.state !== 'disconnected' || p.entries.length);
  const rows = merge(connected, filter).slice(0, 50);
  // A connected place whose last read failed reports `disconnected` with its connection kept, and can be read again.
  const unsettled = failed || read.some(p => p.state === 'unavailable' || p.state === 'denied' || p.state === 'revoked' || p.state === 'disconnected' && p.connection !== null);
  // "No runs" needs every connected place read, and a read that succeeded or runs Bridge already holds; the connect
  // prompt needs no connection at all. Otherwise the place notes say what failed and the section offers Check runs again.
  const answered = !unsettled && read.some(p => p.entries.length > 0 || p.state === 'complete' || p.state === 'partial');
  const linked = read.some(p => p.connection !== null);
  const note = (p: PlaceActivity) => {
    const name = labels.get(p.key);
    if (p.state === 'denied') return `Can't list runs on ${name}: this account isn't allowed to read Tasks.`;
    if (p.state === 'unavailable') return `Couldn't read runs on ${name} just now.${p.entries.length ? ' Showing what Bridge last saw.' : ''}`;
    if (p.state === 'partial') return `${name} has more runs than Bridge reads at once. Orka lists runs by name, so the newest may be missing.`;
    if (p.state === 'revoked') return `Connect to ${name} again to see its runs.`;
    if (p.state === 'disconnected' && p.entries.length) return `${name} isn't connected now. Showing what Bridge last saw.`;
  };
  const notes = read.map(p => [p.key, note(p)] as const).filter((n): n is readonly [string, string] => !!n[1]);
  const retry = failed || read.some(p => p.state === 'unavailable' || p.state === 'denied' || p.state === 'disconnected' && p.connection !== null);
  return <section className="overview-activity" aria-label="Recent activity">
    <header className="overview-activity-head">
      <h2>Recent activity</h2>
      <span className="overview-activity-sub">runs on connected clusters, newest first</span>
      <div className="view-switch overview-activity-filter" role="group" aria-label="Show runs">
        {filters.map(f => <button key={f.id} type="button" aria-pressed={filter === f.id} onClick={() => setFilter(f.id)}>{f.label}</button>)}
      </div>
    </header>
    <p className="overview-note">Runs on this computer aren't listed here yet.</p>
    {failed && <p className="overview-note" role="status">Couldn't read recent activity just now.{activity && ' Showing what Bridge last saw.'}</p>}
    {notes.map(([key, text]) => <p key={key} className="overview-note" role="status">{text}</p>)}
    {retry && <div className="overview-note"><Button kind="quiet" onClick={onCheck}>Check runs again</Button></div>}
    {!activity ? places && !failed && <p className="overview-note" role="status">Reading recent runs…</p>
      : rows.length ? <ol className="overview-activity-list">{rows.map(row => <Entry key={`${row.placeKey}/${row.connection}/${row.uid}`} row={row} place={labels.get(row.placeKey)!} agentShown={known.has(`${row.placeKey}/${row.agentUid}`)} onShowAgent={onShowAgent} />)}</ol>
      : answered ? <p className="overview-note">{emptyFilter[filter]}</p>
      : !linked && <p className="overview-note">Connect a cluster to see its runs here.</p>}
  </section>;
}
