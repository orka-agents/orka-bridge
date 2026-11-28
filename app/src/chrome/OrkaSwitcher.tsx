import { type KeyboardEvent, type ReactNode, useEffect, useId, useRef, useState } from 'react';
import { type ClusterPlaceSnapshot, clusterPlaces, type SelectedPlace, selectCluster } from '../clusters/api';
import type { Place } from '../overview/api';
import './switcher.css';

// Cluster access lives only in native memory, so a remembered cluster can be switched to directly only when
// this run already confirmed it: native's current selection, or one cluster_select accepted earlier.
const confirmed = new Set<string>();
// Find an Orka is always offered; it takes the focus on opening only once the list is long.
const focusFindAbove = 6;

type Row = { id: string; kind: 'computer' } | { id: string; kind: 'cluster'; place: Place; connected: boolean } | { id: string; kind: 'add' } | { id: string; kind: 'manage' };

/** The footer's place button and its menu. Choosing never reconnects a cluster: one not confirmed in this run opens in Clusters. */
export default function OrkaSwitcher({ children, title, place, accessLost = false, computerLine, computerTone, onPlace, onClusters, onManage }: {
  children: ReactNode; title?: string; place: SelectedPlace; computerLine?: string; computerTone?: string;
  /** The cluster in use can no longer authenticate and needs reconnecting. */
  accessLost?: boolean;
  onPlace: (place: SelectedPlace) => void;
  /** Opens Clusters on that cluster's card, or on Add a cluster without one. */
  onClusters: (place?: Place) => void;
  onManage: () => void;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const [snapshot, setSnapshot] = useState<ClusterPlaceSnapshot>();
  const [unavailable, setUnavailable] = useState(false);
  const [query, setQuery] = useState('');
  const button = useRef<HTMLButtonElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const filter = useRef<HTMLInputElement>(null);
  const opening = useRef(0);
  const initialFocus = useRef<'current' | 'first' | 'last'>('current');

  const clusters = snapshot?.places ?? [];
  const filtering = clusters.length + 1 > focusFindAbove;
  const needle = query.trim().toLowerCase();
  const matches = (...values: string[]) => !needle || values.some(value => value.toLowerCase().includes(needle));
  const currentKey = place.kind === 'cluster' && place.target ? snapshot?.selectedKey ?? null : null;
  const rows: Row[] = [
    ...(matches('This computer') ? [{ id: `${id}-computer`, kind: 'computer' as const }] : []),
    ...clusters.filter(item => matches(item.context, item.service)).map((item, index) => ({
      id: `${id}-cluster-${index}`, kind: 'cluster' as const, place: item,
      connected: (item.key === snapshot?.selectedKey || confirmed.has(item.key)) && !(item.key === snapshot?.selectedKey && (accessLost || snapshot.accessLost)),
    })),
    { id: `${id}-add`, kind: 'add' as const }, { id: `${id}-manage`, kind: 'manage' as const },
  ];
  const current = (row: Row) => row.kind === 'computer' ? place.kind === 'thisComputer' : row.kind === 'cluster' && row.place.key === currentKey;

  const items = () => [...(menu.current?.querySelectorAll<HTMLElement>('[role^="menuitem"]') ?? [])];
  // The whole status card opens the menu, not only the name row: a click on its status lines presses the button.
  useEffect(() => {
    const card = button.current?.closest('.status-bar');
    if (!card) return;
    const press = (event: Event) => {
      const target = event.target as Node;
      if (button.current?.contains(target) || menu.current?.contains(target)) return;
      button.current?.click();
    };
    card.addEventListener('click', press);
    return () => card.removeEventListener('click', press);
  }, []);
  useEffect(() => {
    if (!open) return;
    const list = items();
    const target = filtering && initialFocus.current === 'current' ? filter.current
      : initialFocus.current === 'last' ? list.at(-1)
        : initialFocus.current === 'first' ? list[0]
          : list.find(item => item.getAttribute('aria-checked') === 'true') ?? list[0];
    target?.focus();
    // Focus is placed once per opening; filtering re-renders must not move it.
  }, [open, filtering]);
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => { const target = event.target as Node; if (!root.current?.contains(target) && !button.current?.closest('.status-bar')?.contains(target)) close(false); };
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [open]);

  function close(focus: boolean) {
    opening.current++;
    setOpen(false); setQuery('');
    if (focus) button.current?.focus();
  }
  async function show(focus: 'current' | 'first' | 'last') {
    const request = ++opening.current;
    initialFocus.current = focus;
    // Retained views only: this reads no cluster and renews no access.
    const value = await clusterPlaces();
    if (request !== opening.current) return;
    if (value?.selectedKey) confirmed.add(value.selectedKey);
    setSnapshot(value); setUnavailable(!value); setOpen(true);
  }
  async function choose(row: Row) {
    close(true);
    if (row.kind === 'add') return onClusters();
    if (row.kind === 'manage') return onManage();
    // The place in use whose access needs reconnecting opens in Clusters to be connected again.
    if (current(row) && !(row.kind === 'cluster' && !row.connected)) return;
    if (row.kind === 'computer') return onPlace({ kind: 'thisComputer' });
    if (row.connected) {
      try {
        const selected = await selectCluster(row.place.key);
        confirmed.add(row.place.key);
        const provider = selected.providers.find(p => p.ready && place.kind === 'cluster' && place.target?.id === selected.target.id && p.id === place.providerId)
          ?? selected.providers.find(p => p.ready);
        return onPlace({ kind: 'cluster', ...selected, providerId: provider?.id });
      } catch {
        // A refused selection also clears native's previous one, so a cluster in use is no longer usable.
        confirmed.delete(row.place.key);
        if (place.kind === 'cluster') onPlace({ kind: 'cluster' });
      }
    }
    onClusters(row.place);
  }
  function move(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(true); return; }
    // Tab leaves from the button's position, so the next stop is the one after the switcher.
    if (event.key === 'Tab') { close(true); return; }
    const list = items();
    const at = list.indexOf(document.activeElement as HTMLElement);
    const next = event.key === 'ArrowDown' ? (at + 1) % list.length : event.key === 'ArrowUp' ? at <= 0 ? list.length - 1 : at - 1
      : event.key === 'Home' && at >= 0 ? 0 : event.key === 'End' && at >= 0 ? list.length - 1 : undefined;
    if (next !== undefined) { event.preventDefault(); list[next]?.focus(); return; }
    if (at >= 0 && event.key.length === 1 && event.key !== ' ' && !event.ctrlKey && !event.metaKey && !event.altKey) filter.current?.focus();
  }

  const renderRow = (row: Row) => {
    if (row.kind === 'add' || row.kind === 'manage') {
      return <button key={row.id} type="button" role="menuitem" className="switcher-item switcher-action" onClick={() => { void choose(row); }}>
        <span className="switcher-mark" aria-hidden="true">{row.kind === 'add' && <svg viewBox="0 0 16 16"><path d="M8 3v10M3 8h10" /></svg>}</span>
        <span className="switcher-name">{row.kind === 'add' ? 'Add a cluster' : 'Manage clusters'}</span>
      </button>;
    }
    const checked = current(row);
    const tone = row.kind === 'computer' ? computerTone : row.connected ? 'ready' : undefined;
    const meta = row.kind === 'computer' ? computerLine : row.connected ? <><code>{row.place.service.replace(/\s*\/\s*/g, '/')}</code> · namespace {row.place.namespace}</> : checked ? 'Needs reconnecting' : 'Connect again';
    return <button key={row.id} type="button" role="menuitemradio" aria-checked={checked} className="switcher-item" aria-labelledby={`${row.id}-name`} aria-describedby={meta ? `${row.id}-meta` : undefined} onClick={() => { void choose(row); }}>
      <span className={tone ? `switcher-dot ${tone}` : 'switcher-dot'} aria-hidden="true" />
      <span className="switcher-copy">
        <span className="switcher-name" id={`${row.id}-name`}>{row.kind === 'computer' ? 'This computer' : <code>{row.place.context}</code>}</span>
        {meta && <span className="switcher-meta" id={`${row.id}-meta`}>{meta}</span>}
      </span>
      <span className="switcher-check" aria-hidden="true">{checked && <svg viewBox="0 0 16 16"><path d="m3.5 8.5 3 3 6-7" /></svg>}</span>
    </button>;
  };
  const computer = rows.filter(row => row.kind === 'computer');
  const remembered = rows.filter(row => row.kind === 'cluster');
  return <div className="orka-switcher" ref={root}>
    <button ref={button} type="button" className="host-chip" title={title} aria-haspopup="menu" aria-expanded={open} aria-controls={open ? `${id}-menu` : undefined}
      onClick={() => { if (open) close(false); else void show('current'); }}
      onKeyDown={event => {
        if (!open && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) { event.preventDefault(); void show(event.key === 'ArrowDown' ? 'first' : 'last'); }
      }}>
      {children}
      <svg className="switcher-chevron" viewBox="0 0 16 16" aria-hidden="true"><path d="m5 6 3-3 3 3M5 10l3 3 3-3" /></svg>
    </button>
    {open && <div className="switcher-popover" onKeyDown={move}>
      <div className="switcher-filter">
        <svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="7" cy="7" r="4.5" /><path d="m10.5 10.5 3 3" /></svg>
        <input ref={filter} type="search" aria-label="Find an Orka" placeholder="Find an Orka" value={query} onChange={event => setQuery(event.target.value)} />
      </div>
      <div ref={menu} id={`${id}-menu`} role="menu" aria-label="Orka in use">
        {computer.length > 0 && <div role="group" aria-labelledby={`${id}-computer-label`}>
          <p className="switcher-group-label" id={`${id}-computer-label`} role="presentation">On this computer</p>
          {computer.map(renderRow)}
        </div>}
        {(remembered.length > 0 || unavailable) && <div role="group" aria-labelledby={`${id}-clusters-label`}>
          <p className="switcher-group-label" id={`${id}-clusters-label`} role="presentation">Clusters</p>
          {remembered.map(renderRow)}
          {unavailable && <p className="switcher-note" role="presentation">Couldn't read your clusters.</p>}
        </div>}
        {needle && computer.length === 0 && remembered.length === 0 && <p className="switcher-note" role="presentation">No cluster matches.</p>}
        <div role="separator" className="switcher-separator" />
        {rows.filter(row => row.kind === 'add' || row.kind === 'manage').map(renderRow)}
      </div>
    </div>}
  </div>;
}
