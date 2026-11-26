import { useEffect, useState } from 'react';
import { Button } from '../components/Button';
import { SAMPLE_DOCUMENT_ID, agentCall } from '../designer/agent-api';
import { readForm } from '../designer/agent-form';
import TryTask from '../designer/TryTask';
import { type AgentEntry, listDocuments } from '../designer/documents-api';
import './home.css';

// Home is a place to give one of your agents a task. It runs once in the workspace on this computer, and its
// result is shared with that agent's Try a task view in Designer.
export default function Home({ active, revision, blocked: placeBlocked, blockedFor, onOpenAgent, onChooseWorkspace }: {
  active: boolean; revision: number; blocked?: string; blockedFor?: (documentId: string) => string | undefined;
  onOpenAgent: (documentId: string) => void; onChooseWorkspace: () => void;
}) {
  // null means the list could not be read, which is different from having no agents.
  const [agents, setAgents] = useState<AgentEntry[] | null>();
  const [documentId, setDocumentId] = useState<string>();
  const [attempt, setAttempt] = useState(0);
  const [model, setModel] = useState<{ id: string; model?: string }>();
  useEffect(() => {
    if (!active) return;
    let live = true;
    void listDocuments().then(list => { if (live) setAgents(list ? list.filter(agent => agent.available) : null); });
    return () => { live = false; };
  }, [active, revision, attempt]);
  const chosen = agents?.find(agent => agent.id === documentId) ?? agents?.find(agent => agent.id === SAMPLE_DOCUMENT_ID) ?? agents?.[0];
  const chosenId = chosen?.id;
  // The saved model, for the result card only. An unreadable file just leaves it out.
  useEffect(() => {
    if (!active || !chosenId) return;
    let live = true;
    void agentCall('agent_open', undefined, chosenId).then(pair => { if (live) setModel({ id: chosenId, model: readForm(pair.agent).model }); }, () => undefined);
    return () => { live = false; };
  }, [active, chosenId, revision]);
  const blocked = placeBlocked ?? (chosen ? blockedFor?.(chosen.id) : undefined);
  return <section className="home" aria-labelledby="page-title">
    <div className="home-hero">
      <div className="art-orca" aria-hidden="true" />
      <h1 id="page-title">What do you want done?</h1>
      <p className="muted">Give one of your agents a task. It runs in your workspace on this computer.</p>
    </div>
    {agents === undefined ? <p className="muted" role="status">Reading your agents…</p>
      : agents === null ? <p className="home-blocked" role="alert">Bridge couldn’t read your agents. <Button type="button" kind="secondary" onClick={() => { setAgents(undefined); setAttempt(value => value + 1); }}>Try again</Button></p>
      : !chosen ? <p className="muted">You have no agents yet. Create one in Designer.</p>
      : <>
        <div className="home-agent">
          <label htmlFor="home-agent">Ask</label>
          <select id="home-agent" className="field" value={chosen.id} onChange={event => setDocumentId(event.target.value)}>
            {agents.map(agent => <option key={agent.id} value={agent.id}>{agent.name}</option>)}
          </select>
          <Button type="button" kind="quiet" onClick={() => onOpenAgent(chosen.id)}>Open in Designer</Button>
        </div>
        {blocked && <p className="home-blocked">{blocked} {placeBlocked && <Button type="button" kind="quiet" onClick={onChooseWorkspace}>Open Clusters</Button>}</p>}
        {active && <TryTask key={chosen.id} documentId={chosen.id} agentName={chosen.name} model={model?.id === chosen.id ? model.model : undefined} blocked={blocked} active={active} />}
      </>}
  </section>;
}
