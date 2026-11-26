import { useId, useState } from 'react';
import { Button } from '../components/Button';
import { StatusPill } from '../components/DesignSystem';
import { agentCall } from '../designer/agent-api';
import { addTool, changeDescription, changeInstructions } from '../designer/agent-form';
import { builtInTools } from '../designer/built-in-tools';
import { type AgentEntry, createDocument } from '../designer/documents-api';
import { modelLabel } from '../designer/model-label';
import type { AgentProposal } from './cards';
import { type CardRecord, updateCard } from './conversation-store';

// New agents start on Bridge's default model (agent_folder.rs blank_agent); Designer changes it.
const newAgentModel = 'gpt-5.6-terra';
const validName = (name: string) => /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(name);

// Create agent writes an ordinary agent file in Bridge's folder: agent_create, then agent_save with the draft.
export default function AgentCard({ cardKey, proposal, record, actionable, agents, onCreated, onOpenAgent }: {
  cardKey: string; proposal: AgentProposal; record: CardRecord; actionable: boolean; agents: AgentEntry[];
  onCreated: () => void; onOpenAgent: (documentId: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  if (record.status === 'editing' && actionable) return <EditAgent cardKey={cardKey} proposal={proposal} agents={agents} />;
  const created = record.status === 'created' ? record.agent : undefined;
  const taken = agents.some(agent => agent.name === proposal.name);
  const open = actionable && record.status === 'proposed';
  const create = async () => {
    setBusy(true); setError(undefined);
    let documentId: string | undefined;
    try {
      documentId = await createDocument(proposal.name);
      const pair = await agentCall('agent_open', undefined, documentId);
      let draft = changeInstructions(changeDescription(pair.agent, proposal.description), proposal.instructions);
      for (const tool of proposal.tools) draft = addTool(draft, tool);
      await agentCall('agent_save', draft, documentId, pair.source);
      updateCard(cardKey, current => ({ ...current, status: 'created', agent: { documentId: documentId!, name: proposal.name } }));
    } catch (reason) {
      if (documentId) {
        // The file exists but holds a blank draft: say so, and offer it rather than a second create.
        updateCard(cardKey, current => ({ ...current, status: 'created', agent: { documentId: documentId!, name: proposal.name } }));
        setError('The agent file was created, but its draft could not be written. Open it in Designer to finish it.');
      } else setError(reason instanceof Error ? reason.message : 'Your agent could not be created.');
    } finally {
      setBusy(false);
      if (documentId) onCreated();
    }
  };
  return <article className="card nav-card" aria-label={created ? `Agent ${proposal.name}: Created` : 'Proposed agent'}>
    <header className="nav-card-head"><h3 className="section-label">{created ? 'Agent' : 'Proposed agent'}</h3>
      {created ? <StatusPill state="passed">Created</StatusPill> : <StatusPill state="not-started">Not created</StatusPill>}</header>
    <dl className="nav-rows">
      <div><dt>Name</dt><dd><code>{proposal.name}</code></dd></div>
      <div><dt>Description</dt><dd>{proposal.description}</dd></div>
      <div><dt>Instructions</dt><dd className="nav-task-text">{proposal.instructions}</dd></div>
      <div><dt>It can use</dt><dd>{proposal.tools.length ? proposal.tools.map((tool, index) => <span key={tool}>{index ? ', ' : ''}{builtInTools[tool]} <code>{tool}</code></span>) : 'No tools'}</dd></div>
      <div><dt>Thinks with</dt><dd>{modelLabel(newAgentModel)}. You can change it in Designer.</dd></div>
      <div><dt>Kept in</dt><dd>Bridge’s folder, as <code>{proposal.name}.agent.md</code></dd></div>
      <div><dt>First test</dt><dd>Starts empty. Finish it in Designer before running tests or sending.</dd></div>
    </dl>
    {error && <div className="alert alert-failed"><p className="alert-title" role="alert">{error}</p></div>}
    {created ? <div className="nav-actions"><Button type="button" kind="primary" onClick={() => onOpenAgent(created.documentId)}>Open in Designer</Button></div>
      : open && <>
        <div className="nav-actions">
          <Button type="button" kind="primary" disabled={busy || taken} aria-busy={busy ? 'true' : undefined} title={taken ? 'Choose another name with Edit.' : undefined} onClick={() => { void create(); }}>Create agent</Button>
          <Button type="button" kind="secondary" disabled={busy} onClick={() => updateCard(cardKey, current => ({ ...current, status: 'editing' }))}>Edit</Button>
          <Button type="button" kind="quiet" disabled={busy} onClick={() => updateCard(cardKey, current => ({ ...current, status: 'dismissed' }))}>Cancel</Button>
        </div>
        {taken && <p className="muted nav-reason">You already have an agent named {proposal.name}. Choose another name with Edit.</p>}
      </>}
  </article>;
}

function EditAgent({ cardKey, proposal, agents }: { cardKey: string; proposal: AgentProposal; agents: AgentEntry[] }) {
  const id = useId();
  const [name, setName] = useState(proposal.name);
  const [description, setDescription] = useState(proposal.description);
  const [instructions, setInstructions] = useState(proposal.instructions);
  const taken = agents.some(agent => agent.name === name);
  const nameError = !validName(name) ? 'Use lowercase letters, digits and hyphens, up to 63 characters.' : taken ? 'You already have an agent with this name.' : undefined;
  const valid = !nameError && description.trim().length > 0 && description.length <= 200 && !/[\r\n]/.test(description) && instructions.trim().length > 0 && instructions.length <= 8000;
  return <article className="card card-editing nav-card" aria-label="Proposed agent">
    <header className="nav-card-head"><h3 className="section-label">Proposed agent</h3><StatusPill state="running">Editing</StatusPill></header>
    <label className="nav-field" htmlFor={`${id}-name`}><span>Name</span>
      <input id={`${id}-name`} className="field nav-mono" value={name} aria-invalid={nameError ? 'true' : undefined} aria-describedby={nameError ? `${id}-name-error` : undefined} onChange={event => setName(event.target.value)} />
    </label>
    {nameError && <p className="field-error" id={`${id}-name-error`}>{nameError}</p>}
    <label className="nav-field" htmlFor={`${id}-description`}><span>Description</span>
      <input id={`${id}-description`} className="field" value={description} onChange={event => setDescription(event.target.value)} />
    </label>
    <label className="nav-field" htmlFor={`${id}-instructions`}><span>Instructions</span>
      <textarea id={`${id}-instructions`} className="field" rows={4} value={instructions} onChange={event => setInstructions(event.target.value)} />
    </label>
    <div className="nav-actions">
      <Button type="button" kind="primary" disabled={!valid} onClick={() => updateCard(cardKey, current => ({ ...current, status: 'proposed', edited: { ...proposal, name, description, instructions } }))}>Save changes</Button>
      <Button type="button" kind="secondary" onClick={() => updateCard(cardKey, current => ({ ...current, status: 'proposed' }))}>Discard changes</Button>
    </div>
  </article>;
}
