import { useEffect, useRef, useState } from 'react';
import { contexts, discover, connectCluster, type ContextView, type TargetView, type AccessView, type SelectedPlace } from './api';
import './clusters.css';

const messages = {
  needServiceAccount: 'This Orka needs an existing ServiceAccount. Ask your cluster administrator for its name.',
  denied: 'Access was denied. Ask your cluster administrator to check your permissions.',
  unavailable: 'Access could not be confirmed. Check your existing sign-in, credential plugin and Orka connection, then try again.',
  unsupportedCredential: 'This context’s credentials could not be used. Check its sign-in method and credential plugin outside Bridge, then try again.',
};
export default function ClusterCard({ onPlace, onOpenAgent }: { onPlace: (place: SelectedPlace) => void; onOpenAgent: () => void }) {
  const [choices, setChoices] = useState<ContextView[]>([]);
  const [contextId, setContextId] = useState('');
  const [targets, setTargets] = useState<TargetView[]>();
  const [targetId, setTargetId] = useState('');
  const [access, setAccess] = useState<AccessView>();
  const [providerId, setProviderId] = useState('');
  const [account, setAccount] = useState('');
  const [needsAccount, setNeedsAccount] = useState(false);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string>();
  const [retry, setRetry] = useState(0);
  const [details, setDetails] = useState(false);
  const epoch = useRef(0); const admitted = useRef(false);
  const accountField = useRef<HTMLInputElement>(null);
  const providerField = useRef<HTMLSelectElement>(null);
  useEffect(() => {
    let mounted = true;
    void contexts().then(v => { if (mounted) { setChoices(v); setBusy(false); } }, () => {
      if (mounted) { setError('kubectl contexts could not be read. Check your kubeconfig, then try again.'); setBusy(false); }
    });
    return () => { mounted = false; epoch.current++; };
  }, [retry]);
  useEffect(() => {
    if (access?.status === 'needServiceAccount') accountField.current?.focus();
    if (access?.status === 'connected') providerField.current?.focus();
  }, [access]);
  const target = targets?.find(t => t.id === targetId);
  async function check(id: string) {
    const request = ++epoch.current;
    setContextId(id); setTargets(undefined); setTargetId(''); setAccess(undefined); setAccount(''); setNeedsAccount(false); setError(undefined); setDetails(false);
    onPlace({ kind: 'cluster' });
    if (!id) { setBusy(false); return; }
    setBusy(true);
    try {
      const found = await discover(id);
      if (request !== epoch.current) return;
      setTargets(found); setTargetId(found.length === 1 ? found[0].id : '');
    } catch (error) { if (request === epoch.current) setError(error instanceof Error ? error.message : 'Orka could not be checked on this context.'); }
    finally { if (request === epoch.current) setBusy(false); }
  }
  async function confirm() {
    if (!target || busy || admitted.current) return;
    admitted.current = true; const request = ++epoch.current; setBusy(true); setError(undefined);
    try {
      const value = await connectCluster(target.id, needsAccount ? account.trim() : null);
      if (request !== epoch.current) return;
      setAccess(value);
      if (value.status === 'needServiceAccount') setNeedsAccount(true);
      const selectedProvider = value.providers.length === 1 ? value.providers[0].id : '';
      setProviderId(selectedProvider);
      onPlace(value.status === 'connected' ? { kind: 'cluster', target, providers: value.providers, providerId: selectedProvider } : { kind: 'cluster' });
    } catch { if (request === epoch.current) setError('Access could not be confirmed. Try again.'); }
    finally { admitted.current = false; if (request === epoch.current) setBusy(false); }
  }
  return <section className="cluster-card" aria-label="Connect an existing cluster" aria-busy={busy}>
    <label htmlFor="cluster-context">kubectl context</label>
    <select id="cluster-context" value={contextId} disabled={busy} onChange={event => { void check(event.target.value); }}>
      <option value="">Choose a context</option>{choices.map(c => <option key={c.id} value={c.id}>{c.name}</option>)}
    </select>
    {!busy && !error && !choices.length && <p>No kubectl contexts were found. Add your existing context to kubeconfig, then quit and reopen Bridge.</p>}
    {busy && <p role="status">{!choices.length ? 'Reading kubectl contexts…' : targets ? 'Confirming access…' : 'Checking for Orka…'}</p>}
    {targets?.length === 0 && <p role="status">No Orka was found on this context.</p>}
    {targets && targets.length > 1 && <><label htmlFor="cluster-installation">Orka installation</label>
      <select id="cluster-installation" disabled={busy} value={targetId} onChange={e => { setTargetId(e.target.value); setAccess(undefined); setAccount(''); setNeedsAccount(false); onPlace({ kind: 'cluster' }); }}>
        <option value="">Choose an installation</option>{targets.map(t => <option key={t.id} value={t.id}>{t.service_namespace} / {t.service_name} · {t.watched_namespace}</option>)}
      </select></>}
    {target && <>
      <div className="cluster-found"><b>Orka found</b><span>{target.image}</span></div>
      <p className="muted">Use confirms access to this Orka. It does not create an Agent or run checks.</p>
      {needsAccount && access?.status !== 'connected' && <><p>{messages.needServiceAccount}</p>
        <label htmlFor="cluster-account">ServiceAccount name</label><input ref={accountField} id="cluster-account" autoComplete="off" spellCheck={false} disabled={busy} value={account} onChange={e => setAccount(e.target.value)} />
        <p className="muted">An existing account in {target.watched_namespace}. Bridge does not create accounts or permissions.</p></>}
      {access && access.status !== 'connected' && access.status !== 'needServiceAccount' && <p role="alert">{messages[access.status]}</p>}
      {access?.status === 'connected' ? <p role="status">Access confirmed</p> : <button type="button" className="workspace-primary" disabled={busy || (needsAccount && !account.trim())} onClick={() => { void confirm(); }}>Use this cluster</button>}
      <button type="button" className="workspace-disclosure" aria-expanded={details} aria-controls="cluster-details" onClick={() => setDetails(v => !v)}>{details ? 'Hide details' : 'Show details'}</button>
      {details && <dl id="cluster-details" className="cluster-details"><div><dt>Context</dt><dd>{target.context_name}</dd></div><div><dt>Service</dt><dd>{target.service_namespace} / {target.service_name}</dd></div><div><dt>Watched namespace</dt><dd>{target.watched_namespace}</dd></div><div><dt>Orka image</dt><dd>{target.image}</dd></div>
        {access?.providers.map(p => <div key={p.id}><dt>Provider</dt><dd>{p.name} <span className="muted">{p.ready ? 'Ready' : 'Not ready'}</span></dd></div>)}</dl>}
      {access?.status === 'connected' && <><label htmlFor="send-provider">Provider</label>
        <select ref={providerField} id="send-provider" value={providerId} onChange={event => { setProviderId(event.target.value); onPlace({ kind: 'cluster', target, providers: access.providers, providerId: event.target.value }); }}>
          <option value="">Choose a Provider</option>{access.providers.map(p => <option key={p.id} value={p.id}>{p.name} · {p.ready ? 'Ready' : 'Not ready'}</option>)}
        </select><p className="muted">The agent keeps the model named in its file. The checks test whether this Provider can serve it.</p>
        <button type="button" className="workspace-primary" disabled={!providerId} title={!providerId ? 'Choose a Provider first' : undefined} onClick={onOpenAgent}>Open your agent</button></>}
    </>}
    {error && <p role="alert">{error}</p>}
    {(error || access?.status === 'unsupportedCredential') && <p className="muted">After changing kubeconfig, quit and reopen Bridge to read it again.</p>}
    {!busy && (error || !choices.length) && <button type="button" className="workspace-secondary" onClick={() => { if (contextId) void check(contextId); else { setBusy(true); setError(undefined); setRetry(v => v + 1); } }}>Try again</button>}
  </section>;
}
