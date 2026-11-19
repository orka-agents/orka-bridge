import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { Button } from '../components/Button';
import { Disclosure } from '../components/Disclosure';
import { RelativeTime } from '../components/RelativeTime';
import type { Places } from './api';
import { placeLabels } from './model';
import { merge, type PlaceSessions, readSessions, readTranscript, roleLabel, type Row, type Sessions, type Transcript } from './sessions-api';

/** An expanded conversation, read again whenever its place is read again. A new connection remounts it (see the row key). */
function TranscriptView({ placeKey, name, revision }: { placeKey: string; name: string; revision: number | null }) {
  const [transcript, setTranscript] = useState<Transcript>();
  useEffect(() => {
    let cancelled = false;
    void readTranscript(placeKey, name).then(value => { if (!cancelled) setTranscript(value); });
    return () => { cancelled = true; };
  }, [placeKey, name, revision]);
  if (!transcript) return <p className="overview-activity-meta" role="status">Reading the conversation…</p>;
  if (transcript.state === 'denied') return <p className="overview-activity-meta" role="status">This account can't read this conversation.</p>;
  if (transcript.state === 'revoked') return <p className="overview-activity-meta" role="status">Connect to this cluster again to read the conversation.</p>;
  if (transcript.state !== 'complete' && transcript.state !== 'partial') return <p className="overview-activity-meta" role="status">Bridge couldn't read this conversation.</p>;
  // A partial read with nothing readable is not an empty conversation.
  if (!transcript.messages.length) return <p className="overview-activity-meta" role={transcript.state === 'partial' ? 'status' : undefined}>{transcript.state === 'partial' ? "Bridge couldn't read this conversation's messages." : 'No messages yet.'}</p>;
  return <>
    {transcript.state === 'partial' && <p className="overview-activity-meta">Showing the latest {transcript.messages.length} messages Bridge could read.</p>}
    <ol className="overview-transcript">{transcript.messages.map((m, i) => <li key={i} className={`overview-transcript-${m.role === 'user' ? 'user' : 'other'}`}>
      <p className="overview-activity-meta">{roleLabel(m.role)}{m.at !== null && <> · <RelativeTime value={m.at} /></>}</p>
      {m.text ? <p className="overview-transcript-text">{m.text}{m.cut && '…'}</p> : m.cut && <p className="overview-activity-meta">Content Bridge doesn't show here.</p>}
      {!!m.tools.length && <p className="overview-activity-meta">Called {m.tools.map((t, j) => <span key={j}>{j > 0 && ', '}<code>{t}</code></span>)}</p>}
    </li>)}</ol>
  </>;
}

function Entry({ row, place }: { row: Row; place: string }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const messages = `${row.messageCount} ${row.messageCount === 1 ? 'message' : 'messages'}`;
  // Laid out like a Recent activity row: name and facts, the time, then the disclosure at the end.
  return <li className="overview-activity-entry" aria-label={`${row.name} on ${place}, ${messages}`}>
    <div className="overview-activity-row">
      <div className="overview-activity-name">
        <h3 className="overview-agent-title">{row.name}</h3>
        <p className="overview-activity-meta">on <code>{place}</code>{row.sessionType && <> · {row.sessionType}</>} · {messages} · started <RelativeTime value={row.createdAt} /></p>
        {/* `agent` is set only when Bridge read the Task and it had not finished. */}
        {row.activeTask && <p className="overview-activity-meta">{row.agent ? <>Running <code>{row.activeTask}</code> with <code>{row.agent}</code></> : <>Active Task <code>{row.activeTask}</code></>}</p>}
      </div>
      <span className="overview-time"><RelativeTime value={row.updatedAt} /></span>
      <Disclosure label="Conversation" open={open} onOpenChange={setOpen} controls={id}>{null}</Disclosure>
    </div>
    {open && <div id={id} className="overview-activity-details"><TranscriptView placeKey={row.placeKey} name={row.name} revision={row.readAt} /></div>}
  </li>;
}

/** Sessions on connected clusters, read only. Channel (gateway) Sessions and the local VM aren't listed.
 * `onCheck` rereads Overview's places, which reads conversations again. */
export function SessionList({ active, places, onCheck }: { active: boolean; places?: Places; onCheck: () => void }) {
  const [sessions, setSessions] = useState<Sessions>();
  const [failed, setFailed] = useState(false);
  const request = useRef(0);
  // Read after each places read, one read at a time, as Recent activity does.
  const reading = useRef(false);
  const queued = useRef(false);
  const start = useCallback(function next() {
    if (reading.current) { queued.current = true; return; }
    reading.current = true;
    const current = ++request.current;
    void readSessions().then(value => {
      reading.current = false;
      // A superseded response is dropped, but a read queued behind it still runs. Unmounting clears the queue.
      if (current === request.current) {
        setFailed(!value);
        if (value) setSessions(previous => ({
          // An unreadable place keeps what Bridge last read on the same connection, and a refusal stands until a
          // Sessions read succeeds. A denied or revoked place, or a new connection, drops its conversations: access
          // was refused or replaced, not merely interrupted.
          places: value.places.map(p => {
            const prior = previous?.places.find(q => q.key === p.key);
            const keep = (p.state === 'unavailable' || p.state === 'disconnected') && p.connection !== null && prior?.connection === p.connection;
            if (keep && prior?.state === 'denied') return { ...prior, state: 'denied' as const };
            return keep && prior?.entries.length ? { ...p, entries: prior.entries, observedAt: prior.observedAt, unsupported: prior.unsupported } : p;
          }),
        }));
      }
      if (queued.current) { queued.current = false; next(); }
    });
  }, []);
  // Read once per places read: a revisit with cached places waits for the visit's own places read.
  const readFor = useRef<Places>(undefined);
  useEffect(() => {
    if (active && places && places !== readFor.current) { readFor.current = places; start(); }
    // A hidden tab drops a queued read; the next places read starts a fresh one.
    if (!active) queued.current = false;
  }, [active, places, start]);
  useEffect(() => () => { request.current++; queued.current = false; readFor.current = undefined; }, []);
  const labels = placeLabels(places?.places ?? []);
  const read = (sessions?.places ?? []).filter(p => labels.has(p.key));
  const connected = read.filter(p => p.state !== 'disconnected' || p.entries.length);
  const rows = merge(connected).slice(0, 50);
  const note = (p: PlaceSessions) => {
    const name = labels.get(p.key);
    if (p.state === 'denied') return `Can't list conversations on ${name}: this account isn't allowed to read Sessions.`;
    if (p.state === 'unavailable') return `Couldn't read conversations on ${name} just now.${p.entries.length ? ' Showing what Bridge last saw.' : ''}`;
    if (p.state === 'partial') return `${name} has more conversations than Bridge reads at once, so conversations started earlier aren't listed, even if they're still in use.`;
    if (p.state === 'revoked') return `Connect to ${name} again to see its conversations.`;
    if (p.state === 'disconnected' && p.entries.length) return `${name} isn't connected now. Showing what Bridge last saw.`;
  };
  const unsupported = (p: PlaceSessions) => p.unsupported > 0
    && `${p.unsupported === 1 ? '1 conversation' : `${p.unsupported} conversations`} on ${labels.get(p.key)} ${p.unsupported === 1 ? 'has an ID' : 'have IDs'} Bridge can't open yet, so ${p.unsupported === 1 ? "it isn't" : "they aren't"} listed.`;
  const notes = read.flatMap(p => [[p.key, note(p)] as const, [`${p.key}/unsupported`, unsupported(p) || undefined] as const]).filter((n): n is readonly [string, string] => !!n[1]);
  // A connected place whose last read failed reports `disconnected` with its connection kept, and can be read again.
  const retry = failed || read.some(p => p.state === 'unavailable' || p.state === 'denied' || (p.state === 'disconnected' && p.connection !== null));
  return <section className="overview-activity" aria-label="Conversations">
    <header className="overview-activity-head">
      <h2>Conversations</h2>
      <span className="overview-activity-sub">sessions on connected clusters, most recent first</span>
    </header>
    <p className="overview-note">Conversations on this computer, and those started through Telegram, Teams or other channels, aren't listed here yet.</p>
    {failed && <p className="overview-note" role="status">Couldn't read conversations just now.{sessions && ' Showing what Bridge last saw.'}</p>}
    {notes.map(([key, text]) => <p key={key} className="overview-note" role="status">{text}</p>)}
    {retry && <div className="overview-note"><Button kind="quiet" onClick={onCheck}>Check conversations again</Button></div>}
    {!sessions ? places && !failed && <p className="overview-note" role="status">Reading conversations…</p>
      : !connected.length ? <p className="overview-note">Connect a cluster to see its conversations here.</p>
      : !rows.length ? <p className="overview-note">No conversations yet on connected clusters.</p>
      : <ol className="overview-activity-list">{rows.map(row => <Entry key={`${row.placeKey}/${row.connection ?? ''}/${row.name}`} row={row} place={labels.get(row.placeKey)!} />)}</ol>}
  </section>;
}
