import { Disclosure } from '../components/Disclosure';
import { RelativeTime } from '../components/RelativeTime';
import ContextPicker from './ContextPicker';
import ServiceAccountPicker from './ServiceAccountPicker';
import { useEffect, useRef, useState } from 'react';
import { type AccessView, type ClusterFailure, type ContextView, connectCluster, contexts, discover, failedCheck, reloadContexts, reconnectCluster, forgetCluster, clusterPlaces, type SelectedPlace, type TargetView } from './api';
import { clusterDetails, clusterFailureWords, clusterRecovery } from './recovery';
import { Button } from '../components/Button';
import { StatusIcon } from '../components/DesignSystem';
import { readPlaces, type Place } from '../overview/api';
import './clusters.css';

type Failure = { failure: ClusterFailure; operation: 'Read kubeconfig' | 'Reload kubeconfig' | 'Discover Orka' | 'Use cluster' | 'Reconnect cluster' };
type Entry = { revision: number; current?: SelectedPlace; remembered?: Place };
export default function ClusterCard({ active = true, entry, selection, onPlace, onContextName, onOpenAgent, onDone, onRemoved }: { active?: boolean; entry?: Entry; selection?: SelectedPlace; onContextName?: (name: string | undefined) => void; onPlace: (place: SelectedPlace) => void; onOpenAgent: () => void; onDone?: () => void; onRemoved?: (key: string, targetId: string | undefined, navigate: boolean) => void }) {
  const [choices, setChoices] = useState<ContextView[]>([]);
  const [foundContexts, setFoundContexts] = useState<Set<string>>(new Set());
  const [contextId, setContextId] = useState('');
  const [targets, setTargets] = useState<TargetView[]>();
  const [targetId, setTargetId] = useState('');
  const [access, setAccess] = useState<AccessView>();
  const [providerId, setProviderId] = useState('');
  const [account, setAccount] = useState('');
  const [needsAccount, setNeedsAccount] = useState(false);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<Failure>();
  const [reloading, setReloading] = useState(false);
  const [notice, setNotice] = useState('');
  const [details, setDetails] = useState(false);
  const [changingContext, setChangingContext] = useState(false);
  const [retained, setRetained] = useState<Place>();
  const [summary, setSummary] = useState<Place | null>();
  const [persistence, setPersistence] = useState<'complete' | 'unavailable'>();
  const [confirmRemoval, setConfirmRemoval] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [removeError, setRemoveError] = useState(false);
  const removalActive = useRef(false);
  const keepCluster = useRef<HTMLButtonElement>(null);
  const removeButton = useRef<HTMLButtonElement>(null);
  const detailsButton = useRef<HTMLElement>(null);
  const focusRemoval = useRef(false);
  const epoch = useRef(0); const admitted = useRef(false); const reloadActive = useRef(false);
  const reloadInterrupted = useRef(false); const initialLoaded = useRef(false); const wasActive = useRef(active);
  const providerName = useRef('');
  const providerField = useRef<HTMLSelectElement>(null);
  const contextField = useRef<HTMLButtonElement>(null);
  useEffect(() => { if (active && changingContext && !busy && !confirmRemoval) contextField.current?.focus(); }, [active, changingContext, busy, confirmRemoval]);
  function readContexts() {
    const request = ++epoch.current; setBusy(true); setError(undefined);
    void contexts().then(v => { if (request === epoch.current) { setChoices(v); initialLoaded.current = true; } }, error => {
      if (request === epoch.current) { setError({ failure: failedCheck(error, 'configuration'), operation: 'Read kubeconfig' }); initialLoaded.current = false; }
    }).finally(() => { if (request === epoch.current) setBusy(false); });
  }
  useEffect(() => {
    if (!entry) readContexts();
    return () => { epoch.current++; };
  }, []);
  useEffect(() => {
    if (!entry) return;
    // Remembered identity is not a capability. Only Manage receives App's still-current access.
    // Reload clears the choices; a new entry must replace an interrupted reload's fenced reply.
    if (reloadActive.current || reloadInterrupted.current) initialLoaded.current = false;
    epoch.current++; admitted.current = false; reloadActive.current = false; reloadInterrupted.current = false;
    const current = entry.current?.kind === 'cluster' ? entry.current : undefined;
    const target = current?.target;
    setContextId(target?.context_id ?? ''); setTargets(target ? [target] : undefined); setTargetId(target?.id ?? '');
    setAccess(target ? { status: 'connected', providers: current?.providers ?? [], failure: null } : undefined);
    setProviderId(current?.providerId ?? ''); providerName.current = current?.providers?.find(p => p.id === current.providerId)?.name ?? '';
    // A Manage visit may retain the name already entered for this exact target handle.
    if (!target || target.id !== targetId) { setAccount(''); setNeedsAccount(false); }
    setError(undefined); setDetails(false); setNotice(''); setChangingContext(false);
    setRetained(entry.remembered); setSummary(undefined); setPersistence(undefined); setConfirmRemoval(false); setRemoveError(false);
    setReloading(false);
    if (entry.remembered && !target) void reconnect(entry.remembered.key);
    // An entry reset also fences a pending local kubeconfig read; restart just that read.
    else if (!target && !initialLoaded.current) readContexts(); else setBusy(false);
  }, [entry]);
  useEffect(() => {
    if (!active) return;
    if (access?.status === 'connected') providerField.current?.focus();
  }, [access, active]);
  useEffect(() => {
    const was = wasActive.current; wasActive.current = active;
    if (!active) {
      // Retain configuration, not active confirmation; late replies cannot reselect it.
      if (reloadActive.current) reloadInterrupted.current = true;
      epoch.current++; admitted.current = false; reloadActive.current = false;
      setBusy(false); setReloading(false);
      // A completed Setup subview survives tab navigation. Standalone callers keep the
      // original fresh-Use-on-reactivation behavior; pending replies are always fenced.
      if (!entry) setAccess(undefined);
      return;
    }
    // Resume a configuration read interrupted by deactivation; a completed read (success,
    // failure, or one already loaded) is left alone, and late replies to the interrupted
    // request stay discarded by the epoch check above.
    if (!was && reloadInterrupted.current) { reloadInterrupted.current = false; void reload(); }
    else if (!was && entry?.remembered && !(entry.current?.kind === 'cluster' && entry.current.target) && !changingContext && !targets && !error) void reconnect(entry.remembered.key);
    else if (!was && !initialLoaded.current && (!entry?.remembered || changingContext)) readContexts();
  }, [active]);
  useEffect(() => {
    // Opening a local document elsewhere also revokes this subview's confirmation.
    if (selection && (selection.kind !== 'cluster' || !selection.target)) setAccess(undefined);
  }, [selection]);
  const target = targets?.find(t => t.id === targetId);
  const recovery = error && clusterRecovery(error.failure);
  const contextName = target?.context_name ?? retained?.context ?? choices.find(c => c.id === contextId)?.name;
  // The page is named for the cluster once it is in use or remembered; adding one keeps "Add a cluster" until Use succeeds.
  const named = access?.status === 'connected' || !!entry?.remembered || !!retained;
  useEffect(() => { onContextName?.(named ? contextName : undefined); }, [named, contextName, onContextName]);
  const networkFailure = contextName && error?.failure.result === 'connectionFailed' && ['clusterApi', 'orkaConnection'].includes(error.failure.check);
  const connected = access?.status === 'connected';
  const noContexts = !error && !choices.length && !connected && !targets && (!entry?.remembered || changingContext);
  // A remembered or connected cluster's page names its Orka installation under the title, as the list does.
  const service = target ? `${target.service_namespace}/${target.service_name}` : retained?.service.replace(/\s*\/\s*/g, '/');
  const subtitle = (connected || retained && !changingContext) && service;
  // A denied check is answered in its Details, which open with it as the board shows.
  useEffect(() => { if (error?.operation === 'Use cluster' && error.failure.result === 'permissionDenied') setDetails(true); }, [error]);
  const checkedAt = summary?.checkedAt ?? summary?.observedAt ?? retained?.checkedAt ?? retained?.observedAt;
  useEffect(() => {
    if (!active || !connected || removing) return;
    const request = epoch.current;
    let live = true;
    setSummary(undefined);
    void clusterPlaces().then(async snapshot => {
      if (!live || request !== epoch.current) return;
      setPersistence(snapshot?.persistenceState ?? 'unavailable');
      const key = snapshot?.selectedKey;
      setRetained(previous => snapshot?.places.find(p => p.key === key) ?? previous);
      const inventory = key ? await readPlaces(key) : undefined;
      if (live && request === epoch.current) setSummary(inventory?.places.find(p => p.key === key) ?? null);
    });
    return () => { live = false; };
  }, [active, connected, access, entry, removing, changingContext]);
  useEffect(() => {
    if (!active || removing) return;
    if (confirmRemoval) keepCluster.current?.focus();
    else if (focusRemoval.current) { (details || !connected || error ? removeButton.current : detailsButton.current)?.focus(); focusRemoval.current = false; }
  }, [confirmRemoval, active, removing, details, connected, error]);
  async function reconnect(key: string) {
    const request = ++epoch.current; setBusy(true); setError(undefined);
    initialLoaded.current = false; setChoices([]); setFoundContexts(new Set());
    // Native reconnect retires the prior session before resolving fresh handles.
    onPlace({ kind: 'cluster' });
    try {
      const value = await reconnectCluster(key);
      if (request !== epoch.current) return;
      setContextId(value.context_id); setTargets([value]); setTargetId(value.id);
    } catch (error) { if (request === epoch.current) setError({ failure: failedCheck(error, 'configuration'), operation: 'Reconnect cluster' }); }
    finally { if (request === epoch.current) setBusy(false); }
  }
  async function remove() {
    if (!active || !retained || removalActive.current) return;
    const { key } = retained; const removedTarget = target?.id;
    removalActive.current = true; const request = ++epoch.current; admitted.current = false;
    setRemoving(true); setBusy(true); setRemoveError(false);
    try {
      await forgetCluster(key);
      // Removal revokes access even when navigation hid this card while native cleanup ran.
      const current = request === epoch.current;
      if (current) { ++epoch.current; setAccess(undefined); setTargets(undefined); setRetained(undefined); setBusy(false); }
      onRemoved?.(key, removedTarget, current);
    } catch { if (request === epoch.current) setRemoveError(true); }
    finally { removalActive.current = false; setRemoving(false); if (request === epoch.current) setBusy(false); }
  }
  async function reload() {
    if (!active || reloadActive.current) return;
    reloadActive.current = true; initialLoaded.current = false; admitted.current = false; const request = ++epoch.current;
    setChangingContext(true); setReloading(true); setBusy(true); setChoices([]); setFoundContexts(new Set()); setContextId(''); setTargets(undefined); setTargetId('');
    setAccess(undefined); setProviderId(''); providerName.current = ''; setAccount(''); setNeedsAccount(false); setError(undefined); setDetails(false); setNotice('');
    onPlace({ kind: 'cluster' });
    try { const value = await reloadContexts(); if (request === epoch.current) { setChoices(value); initialLoaded.current = true; } }
    catch (error) { if (request === epoch.current) setError({ failure: failedCheck(error, 'configuration'), operation: 'Reload kubeconfig' }); }
    finally { if (request === epoch.current) { reloadActive.current = false; setReloading(false); setBusy(false); } }
  }
  async function copy() {
    if (!error) return;
    try { await navigator.clipboard.writeText(clusterDetails(error.failure, error.operation)); setNotice('Details copied. Private diagnostic text is not included.'); }
    catch { setNotice("Couldn't copy. Share the visible error message for help."); }
  }
  async function check(id: string) {
    if (!active) return;
    const request = ++epoch.current;
    setContextId(id); setTargets(undefined); setTargetId(''); setAccess(undefined); setProviderId(''); providerName.current = ''; setAccount(''); setNeedsAccount(false); setError(undefined); setDetails(false); setNotice('');
    setRetained(undefined); setSummary(undefined); setConfirmRemoval(false); setRemoveError(false);
    // Discovery revokes remote handles, but does not replace a selected local workspace.
    if (selection?.kind !== 'thisComputer') onPlace({ kind: 'cluster' });
    if (!id) { setBusy(false); return; }
    setBusy(true);
    try {
      const found = await discover(id);
      if (request !== epoch.current) return;
      setTargets(found); setTargetId(found.length === 1 ? found[0].id : '');
      setFoundContexts(previous => { const next = new Set(previous); if (found.length) next.add(id); else next.delete(id); return next; });
    } catch (error) { if (request === epoch.current) setError({ failure: failedCheck(error, 'clusterApi'), operation: 'Discover Orka' }); }
    finally { if (request === epoch.current) setBusy(false); }
  }
  async function confirm() {
    if (!active || !target || busy || admitted.current) return;
    admitted.current = true; const request = ++epoch.current; setBusy(true); setError(undefined); setNotice(''); setDetails(false);
    setAccess(undefined);
    // Native Use invalidates the old access before it replies. Do not leave App holding it.
    onPlace({ kind: 'cluster' });
    try {
      const value = await connectCluster(target.id, needsAccount ? account.trim() : null);
      if (request !== epoch.current) return;
      setAccess(value);
      if (value.failure) setError({ failure: value.failure, operation: 'Use cluster' });
      if (value.status === 'needServiceAccount') setNeedsAccount(true);
      // Every successful connect mints fresh opaque provider IDs, so a remembered selection is
      // matched by name against the fresh reply rather than by the now-stale ID.
      const remembered = providerName.current;
      const byName = remembered ? value.providers.find(p => p.name === remembered) : undefined;
      const selectedProvider = byName ? byName.id
        : providerId && value.providers.some(p => p.id === providerId) ? providerId
        : value.providers.length === 1 ? value.providers[0].id : '';
      providerName.current = value.providers.find(p => p.id === selectedProvider)?.name ?? remembered;
      setProviderId(selectedProvider);
      onPlace(value.status === 'connected' ? { kind: 'cluster', target, providers: value.providers, providerId: selectedProvider } : { kind: 'cluster' });
    } catch (error) { if (request === epoch.current) setError({ failure: failedCheck(error, 'orkaConnection'), operation: 'Use cluster' }); }
    finally { if (request === epoch.current) { admitted.current = false; setBusy(false); } }
  }
  return <section className="cluster-card" aria-label="Connect an existing cluster" aria-busy={busy} hidden={!active}>
    {subtitle && <div className="cluster-result"><p>Orka in <code className="code-chip">{subtitle}</code></p></div>}
    {!connected && !entry?.remembered && <h2 className="cluster-card-title">Connect an existing cluster</h2>}
    {((!connected && !entry?.remembered) || changingContext) && <ContextPicker trigger={contextField} choices={choices} value={contextId} found={foundContexts} disabled={busy} active={active} onChoose={id => { setChangingContext(false); void check(id); }} />}
    {!busy && noContexts && <p className="cluster-notice"><StatusIcon state="needs-you" /><span>No kubectl contexts were found. Add your existing context to kubeconfig, then press Reload kubeconfig.</span></p>}
    {busy && <p role="status" className="cluster-checking"><StatusIcon state="running" />{removing ? 'Removing cluster…' : entry?.remembered && !changingContext && !targets ? 'Checking the remembered cluster…' : reloading ? 'Reload pending. Waiting for any current cluster check, then reading kubeconfig…' : !choices.length ? 'Reading kubectl contexts…' : targets ? 'Confirming access…' : 'Checking the cluster…'}</p>}
    {!busy && targets?.length === 0 && <p role="status" className="cluster-notice"><StatusIcon state="needs-you" /><span>No cluster with Orka was found using this context.</span></p>}
    {!connected && !error && targets && targets.length > 1 && <div className="cluster-found"><StatusIcon state="passed" /><b>Cluster found</b></div>}
    {!connected && targets && targets.length > 1 && <><label htmlFor="cluster-installation">Orka installation</label>
      <select className={`field${targetId ? ' cluster-mono-field' : ''}`} id="cluster-installation" disabled={busy} value={targetId} onChange={e => { setTargetId(e.target.value); setAccess(undefined); setProviderId(''); providerName.current = ''; setAccount(''); setNeedsAccount(false); setError(undefined); setNotice(''); onPlace({ kind: 'cluster' }); }}>
        <option value="">Choose an installation</option>{targets.map(t => <option key={t.id} value={t.id}>{t.service_namespace} / {t.service_name} · {t.watched_namespace}</option>)}
      </select></>}
    {error && recovery && <div className={`alert alert-failed cluster-failure${networkFailure ? ' cluster-failure-only' : ''}`} role="alert">
      <p className="alert-title failure-summary">{networkFailure ? `Can't reach ${contextName}. Check your network or VPN.` : recovery.cause}</p>{!networkFailure && <p>{error.operation === 'Read kubeconfig' ? 'Check again to read your contexts. If you changed kubeconfig, use Reload kubeconfig in Details.' : recovery.next}</p>}
      {networkFailure && <p>Your workspace and agent files are kept. Connection checks do not create an agent or run tests.</p>}
    </div>}
    {error && recovery && !networkFailure && <p className="muted">Your workspace and agent files are kept. Connection checks do not create an agent or run tests.</p>}
    {target && <>
      {connected ? <>
        <ul className="cluster-checks"><li><StatusIcon state="passed" /><b>Cluster found</b></li><li><StatusIcon state="passed" /><b role="status">Access confirmed</b></li></ul>
        <h3>AI connection</h3><label className="sr-only" htmlFor="send-provider">AI connection</label>
        <select className="field cluster-mono-field" ref={providerField} id="send-provider" disabled={removing} value={providerId} onChange={event => { const next = event.target.value; setProviderId(next); providerName.current = access.providers.find(p => p.id === next)?.name ?? ''; onPlace({ kind: 'cluster', target, providers: access.providers, providerId: next }); }}>
          <option value="" disabled>Choose an AI connection</option>{access.providers.map(p => <option key={p.id} value={p.id}>{p.name} · {p.ready ? 'Ready' : 'Not ready'}</option>)}
        </select><p className="muted">Send checks whether this AI connection supports your agent's model.</p>
        <p className="muted">{checkedAt != null ? <>Checked <RelativeTime value={checkedAt} inline /></> : 'Last check unavailable'}</p>
        <h3>Agents on this cluster</h3>
        {summary === undefined ? <p role="status">Reading agents…</p> : !summary || summary.stale ? <p>Agents could not be checked. Check access again to retry.</p> : summary.agents.length ? <ul>{summary.agents.map(agent => <li key={agent.uid}><code>{agent.name}</code>{agent.model && <> · <code>{agent.model}</code></>}</li>)}</ul> : <p>No agents on this cluster yet.</p>}
        {persistence === 'unavailable' && <p role="status">Remembered cluster information is unavailable. This connection is usable, but may not be remembered after Bridge closes.</p>}
      </> : <>
        {!error && targets?.length === 1 && <div className="cluster-found"><StatusIcon state="passed" /><b>Cluster found</b><span><code className="code-chip">{target.watched_namespace}</code> · <code className="code-chip">{target.service_namespace} / {target.service_name}</code></span></div>}
        {!error && <p className="muted">Check that you can access this cluster.</p>}
        {needsAccount && <ServiceAccountPicker key={`${entry?.revision}/${target.id}`} target={target} active={active} disabled={busy} value={account} onChange={setAccount} />}
      </>}
    </>}
    {!target && !connected && !error && targets && targets.length > 1 && <p className="muted">Check that you can access this cluster.</p>}
    {!(retained && confirmRemoval) && <div className="workspace-step-actions cluster-actions">
      {error && <Button type="button" kind="primary" size={networkFailure ? 'large' : 'default'} disabled={busy || (error.operation === 'Use cluster' && needsAccount && !account.trim())} onClick={() => {
        if (error.operation === 'Reconnect cluster' && retained) void reconnect(retained.key);
        else if (error.operation === 'Use cluster') void confirm();
        else if (error.operation === 'Discover Orka') void check(contextId);
        else if (error.operation === 'Read kubeconfig') readContexts();
        else void reload();
      }}>Check again</Button>}
      {!error && target && (connected ? <>
        {onDone && <Button type="button" kind="primary" size="large" disabled={removing} onClick={onDone}>Done</Button>}
        <Button type="button" kind="secondary" size="large" disabled={!providerId || removing} title={!providerId ? 'Choose an AI connection first' : undefined} onClick={onOpenAgent}>Open your agent</Button>
      </> : <Button type="button" kind="primary" size="large" disabled={busy || (needsAccount && !account.trim())} onClick={() => { void confirm(); }}>Use this cluster</Button>)}
      {!error && !target && !connected && targets && targets.length > 1 && <Button type="button" kind="primary" size="large" disabled title="Choose an installation first">Use this cluster</Button>}
      {!busy && !error && targets?.length === 0 && (!entry?.remembered || changingContext) && <Button type="button" kind="secondary" size="large" onClick={() => contextField.current?.click()}>Change context</Button>}
      {!connected && entry?.remembered && !changingContext && <Button type="button" kind="quiet" disabled={removing} onClick={() => { setChangingContext(true); if (!initialLoaded.current) readContexts(); }}>Change context</Button>}
      {!error && !connected && <Button type="button" kind={noContexts ? 'primary' : 'quiet'} disabled={reloading || removing} onClick={() => { void reload(); }}>{reloading ? 'Reloading kubeconfig…' : 'Reload kubeconfig'}</Button>}
      {!busy && !error && !connected && !!targets?.length && (!entry?.remembered || changingContext) && <Button type="button" kind="quiet" onClick={() => contextField.current?.click()}>Change context</Button>}
      {retained && (!connected || error) && <Button ref={removeButton} type="button" kind="quiet" className="cluster-remove" onClick={() => { setConfirmRemoval(true); if (target && !error) setDetails(true); }}>Remove</Button>}
    </div>}
    {target && !error && <Disclosure summaryRef={detailsButton} label={details ? 'Hide details' : 'Show details'} open={details} controls="cluster-details" onOpenChange={setDetails}>
      {details && <div id="cluster-details"><dl className="cluster-details"><div><dt>Context</dt><dd><code className="code-chip">{target.context_name}</code></dd></div><div><dt>Service</dt><dd><code className="code-chip">{target.service_namespace} / {target.service_name}</code></dd></div><div><dt>Watched namespace</dt><dd><code className="code-chip">{target.watched_namespace}</code></dd></div><div><dt>Orka image</dt><dd>{target.image}</dd></div>
        {access?.providers.map(p => <div key={p.id}><dt>Provider</dt><dd>{p.name} <span className="muted">{p.ready ? 'Ready' : 'Not ready'}</span></dd></div>)}</dl>
        {connected && !confirmRemoval && <div className="workspace-step-actions">
          <Button type="button" kind="secondary" disabled={busy} onClick={() => { void confirm(); }}>Check access again</Button>
          <Button type="button" kind="quiet" disabled={busy || removing} onClick={() => { setChangingContext(true); if (!initialLoaded.current) readContexts(); }}>Change context</Button>
          <Button type="button" kind="quiet" disabled={reloading || removing} onClick={() => { void reload(); }}>Reload kubeconfig</Button>
          {retained && !confirmRemoval && <Button ref={removeButton} type="button" kind="quiet" className="cluster-remove" onClick={() => { setConfirmRemoval(true); if (target && !error) setDetails(true); }}>Remove</Button>}
        </div>}
      </div>}</Disclosure>}
    {error && <Disclosure label="Details" open={details} onOpenChange={setDetails}>{details && <div className="cluster-error-details">
      {target && !connected && <p className="cluster-found-line">Cluster found: <code className="code-chip">{target.watched_namespace}</code> · <code className="code-chip">{target.service_namespace} / {target.service_name}</code></p>}
      <p>Step: {clusterFailureWords(error.failure).step}</p><p>Result: {clusterFailureWords(error.failure).result}</p>
      <Button type="button" kind="secondary" onClick={() => { void copy(); }}>Copy details for help</Button>
      <Button type="button" kind="quiet" disabled={reloading || removing} onClick={() => { void reload(); }}>Reload kubeconfig</Button>
      <p className="muted">After reloading kubeconfig, choose a context and check access again.</p>
    </div>}</Disclosure>}
    {retained && confirmRemoval && <div role="group" aria-label="Remove cluster confirmation">
      <p>Remove <code>{retained.context}</code>? This only forgets the cluster in Bridge. Its agents and your kubeconfig are kept.</p>
      {removeError && <p role="alert">Couldn't confirm removal from Bridge. Check that its local storage is available, then reopen Bridge. Its agents and your kubeconfig are kept.</p>}
      <div className="workspace-step-actions">
        <Button ref={keepCluster} type="button" kind="primary" disabled={removing} onClick={() => { focusRemoval.current = true; setConfirmRemoval(false); setRemoveError(false); }}>Keep cluster</Button>
        <Button type="button" kind="secondary" className="button-danger" disabled={removing} onClick={() => { void remove(); }}>Remove cluster</Button>
      </div>
    </div>}
    {notice && <p role="status">{notice}</p>}
  </section>;
}
