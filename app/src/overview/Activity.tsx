import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { Button } from '../components/Button';
import { StatusPill } from '../components/DesignSystem';
import { Disclosure } from '../components/Disclosure';
import { RelativeTime } from '../components/RelativeTime';
import { type Activity, type Filter, latest, merge, phaseView, type PlaceActivity, readActivity, readToolCalls, type Row, type ToolCalls } from './activity-api';
import type { Places } from './api';
import { placeLabels } from './model';

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
  if (!calls) return <p className="overview-card-meta" role="status">Reading tool calls…</p>;
  if (calls.state === 'denied') return <p className="overview-card-meta" role="status">This account can't read this run's events.</p>;
  if (calls.state !== 'complete' && calls.state !== 'partial') return <p className="overview-card-meta" role="status">Couldn't read tool calls just now.</p>;
  if (!calls.calls.length && calls.state === 'complete') return <p className="overview-card-meta">No tool calls recorded.</p>;
  return <>
    {calls.calls.length > 0 && <ol className="overview-activity-tools">{calls.calls.map((c, i) => <li key={i}><code>{c.tool}</code>{c.failed && ' · failed'} · <RelativeTime value={c.at} /></li>)}</ol>}
    {calls.state === 'partial' && <p className="overview-card-meta">Some of this run's tool calls aren't shown.</p>}
  </>;
}

function Entry({ row, place, agentShown, onShowAgent }: { row: Row; place: string; agentShown: boolean; onShowAgent: (key: string) => void }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const view = phaseView[row.phase];
  const subject = row.agent ?? row.task;
  return <li className="card overview-activity-entry" aria-label={`${subject} on ${place}, ${view.pill}`}>
    <header className="overview-card-head">
      <h3>{subject}</h3>
      <StatusPill state={view.status}>{view.pill}</StatusPill>
      <span className="overview-card-time"><RelativeTime value={latest(row)} /></span>
    </header>
    <p className="overview-card-meta">on <code>{place}</code> · run <code>{row.task}</code>{row.test && ' · Bridge test'}</p>
    {row.message && <p className="overview-card-description">{row.message}</p>}
    <div className="overview-card-actions">
      {agentShown && row.agentUid && <Button kind="quiet" aria-label={`Show ${subject} on ${place}`} onClick={() => onShowAgent(`${row.placeKey}/${row.agentUid}`)}>Show agent</Button>}
      <Disclosure className="overview-details-toggle" label="Tool calls" open={open} onOpenChange={setOpen} controls={id}>{null}</Disclosure>
    </div>
    {open && <div id={id} className="overview-details"><ToolCallList placeKey={row.placeKey} task={row.task} revision={row.readAt} /></div>}
  </li>;
}

/** Recent runs across connected clusters, read only. The local VM cannot list Tasks yet. */
export function RecentActivity({ active, places }: { active: boolean; places?: Places }) {
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
          // list. A denied or revoked place, or a new connection, drops it: access was refused or replaced.
          places: value.places.map(p => {
            const prior = previous?.places.find(q => q.key === p.key);
            const keep = (p.state === 'unavailable' || p.state === 'disconnected') && p.connection !== null && prior?.connection === p.connection;
            return keep && prior?.entries.length ? { ...p, entries: prior.entries, observedAt: prior.observedAt } : p;
          }),
        }));
      }
      if (queued.current) { queued.current = false; next(); }
    });
  }, []);
  useEffect(() => {
    if (active && places) start();
  }, [active, places, start]);
  useEffect(() => () => { request.current++; queued.current = false; }, []);
  const showAgent = (key: string) => {
    const card = [...document.querySelectorAll<HTMLElement>('[data-card-key]')].find(node => node.dataset.cardKey === key);
    if (card) { card.focus(); card.scrollIntoView?.({ block: 'center' }); }
  };
  const labels = placeLabels(places?.places ?? []);
  const known = new Set(places?.places.flatMap(p => p.agents.map(a => `${p.key}/${a.uid}`)) ?? []);
  const read = (activity?.places ?? []).filter(p => labels.has(p.key));
  const connected = read.filter(p => p.state !== 'disconnected' || p.entries.length);
  const rows = merge(connected, filter).slice(0, 50);
  const note = (p: PlaceActivity) => {
    const name = labels.get(p.key);
    if (p.state === 'denied') return `Can't list runs on ${name}: this account isn't allowed to read Tasks.`;
    if (p.state === 'unavailable') return `Couldn't read runs on ${name} just now.${p.entries.length ? ' Showing what Bridge last saw.' : ''}`;
    if (p.state === 'partial') return `${name} has more runs than Bridge reads at once. Orka lists runs by name, so the newest may be missing.`;
    if (p.state === 'revoked') return `Connect to ${name} again to see its runs.`;
    if (p.state === 'disconnected' && p.entries.length) return `${name} isn't connected now. Showing what Bridge last saw.`;
  };
  const notes = read.map(p => [p.key, note(p)] as const).filter((n): n is readonly [string, string] => !!n[1]);
  return <section className="overview-group overview-activity" aria-label="Recent activity">
    <header className="overview-group-head">
      <h2>Recent activity</h2>
      <span className="overview-group-sub">runs on connected clusters, newest first</span>
      <div className="view-switch overview-activity-filter" role="group" aria-label="Show runs">
        {filters.map(f => <button key={f.id} type="button" aria-pressed={filter === f.id} onClick={() => setFilter(f.id)}>{f.label}</button>)}
      </div>
    </header>
    <p className="overview-note">Runs on this computer aren't listed here yet.</p>
    {failed && <p className="overview-note" role="status">Couldn't read recent activity just now.{activity && ' Showing what Bridge last saw.'}</p>}
    {notes.map(([key, text]) => <p key={key} className="overview-note" role="status">{text}</p>)}
    {!connected.length ? <p className="overview-note">Connect a cluster to see its runs here.</p>
      : !rows.length ? <p className="overview-note">{emptyFilter[filter]}</p>
      : <ol className="overview-activity-list">{rows.map(row => <Entry key={`${row.placeKey}/${row.connection}/${row.uid}`} row={row} place={labels.get(row.placeKey)!} agentShown={known.has(`${row.placeKey}/${row.agentUid}`)} onShowAgent={showAgent} />)}</ol>}
  </section>;
}
