import { StatusPill } from '../components/DesignSystem';
import type { ModelState } from '../setup/Model';
import type { SetupStatus } from '../setup/state';
import type { ClusterPlaceSnapshot, SelectedPlace } from './api';
import type { Place } from '../overview/api';
import './clusters.css';

export default function Places({ snapshot, unavailable, status, model, place, onLocal, onCluster }: {
  snapshot?: ClusterPlaceSnapshot; unavailable: boolean; status?: SetupStatus; model: ModelState; place: SelectedPlace;
  onLocal: () => void; onCluster: (place: Place, selected: boolean) => void;
}) {
  const ready = status?.localVm === 'ready';
  const started = status?.selected === 'localVm';
  const busy = ['preparing', 'installing', 'starting', 'closing'].includes(status?.localVm ?? '');
  const localLabel = !status ? 'Status unavailable' : ready ? 'Running' : status?.localVm === 'failed' ? 'Needs you' : status?.localVm === 'closed' ? 'Local cluster stopped' : status?.localVm === 'cancelled' ? 'Setup paused' : status?.localVm === 'notIntegrated' ? 'Local cluster unavailable' : busy ? status?.localVm === 'closing' ? 'Stopping the local cluster' : 'Setup in progress' : 'Not set up';
  const action = !status ? 'View details' : ready ? 'Manage' : started ? 'Continue setup' : 'Set up';
  return <div className="places-list">
    <section className="card place-row" aria-label="This computer">
      <span className="icon-tile" aria-hidden="true"><svg className="icon" viewBox="0 0 24 24"><rect x="3" y="4" width="18" height="13" rx="2" /><path d="M8 21h8m-4-4v4" /></svg></span>
      <div className="place-summary"><h2>This computer</h2><p>{!status ? 'Local cluster status could not be read' : ready ? 'Local cluster running' : started ? 'Local cluster setup' : 'The local cluster runs here'}</p><p className="muted">{!status ? 'AI status unknown' : model === 'connected' ? 'AI connected' : model === 'queued' ? 'AI connection queued' : 'AI not connected'}</p></div>
      <StatusPill state={ready ? 'passed' : status?.localVm === 'failed' ? 'needs-you' : busy ? 'running' : 'not-started'}>{localLabel}</StatusPill>
      <button type="button" className="workspace-secondary" onClick={onLocal}>{action}</button>
    </section>
    {snapshot?.places.map(item => {
      const selected = place.kind === 'cluster' && !!place.target && snapshot.selectedKey === item.key;
      const observed = item.observedAt !== null;
      const checked = observed ? `Last checked ${new Date(item.observedAt!).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}` : 'Not checked yet';
      return <section className="card place-row" aria-label={item.context} key={item.key}>
        <span className="icon-tile" aria-hidden="true"><svg className="icon" viewBox="0 0 24 24"><path d="m12 3 9 5-9 5-9-5 9-5ZM3 13l9 5 9-5M3 18l9 5 9-5" /></svg></span>
        <div className="place-summary"><h2>{item.context}</h2><p>{item.namespace} · {item.service}</p>{selected && <p className="muted">{checked}</p>}{observed && !item.stale && <p className="muted">{item.agents.length} {item.agents.length === 1 ? 'agent' : 'agents'} at last check</p>}</div>
        <StatusPill state={selected ? 'passed' : 'not-started'}>{selected ? 'Selected for Designer' : checked}</StatusPill>
        <button type="button" className="workspace-secondary" onClick={() => onCluster(item, selected)}>{selected ? 'Manage' : 'Connect'}</button>
      </section>;
    })}
    {unavailable ? <p role="status">Couldn't read your clusters. Try opening Clusters again.</p> : !snapshot ? <p role="status">Reading your clusters…</p> : snapshot.places.length === 0 && <p className="muted">No other clusters added yet. Add a Kubernetes cluster with Orka installed.</p>}
  </div>;
}
