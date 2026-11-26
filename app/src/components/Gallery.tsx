import type { ReactNode } from 'react';
import { Button, CloseIcon, CopyIcon } from './Button';
import { Chip, Details, GeneratedValue, LongCard, type Status, StatusIcon, StatusPill } from './DesignSystem';
import { Disclosure } from './Disclosure';
import './gallery.css';

// Development only: main.tsx loads this for `?scenario=…&gallery` under `npm run dev`. It is never in a production build.
function Item({ label, children }: { label: string; children: ReactNode }) {
  return <div className="gallery-item"><div className="gallery-sample">{children}</div><span className="section-label">{label}</span></div>;
}
function Section({ title, note, children }: { title: string; note?: string; children: ReactNode }) {
  return <section className="gallery-section" aria-label={title}><h2>{title}</h2>{note && <p className="muted">{note}</p>}{children}</section>;
}
const statuses: { state: Status; words: string[]; means: string }[] = [
  { state: 'passed', words: ['Passed', 'Ready', 'Connected'], means: 'Tests met their checks; a connection works' },
  { state: 'running', words: ['Running', 'Installing', 'Checking'], means: 'Work in progress' },
  { state: 'needs-you', words: ['Needs you', 'Stopped', 'Not run since your edit'], means: 'Waiting on the creator' },
  { state: 'failed', words: ['Failed', 'Denied'], means: 'Did not succeed' },
  { state: 'stale', words: ['Result unknown'], means: 'No result was confirmed' },
  { state: 'not-started', words: ['Not started', 'Cancelled'], means: 'Nothing has run yet' },
];

export default function Gallery() {
  return <main className="gallery">
    <h1 className="gallery-title">Components and states</h1>
    <div className="gallery-columns">
      <div className="gallery-column">
        <Section title="Buttons" note="One primary per card or screen.">
          <div className="gallery-row">
            <Item label="Primary"><Button kind="primary">Create task</Button></Item>
            <Item label="Hover"><Button kind="primary" className="gallery-hover">Create task</Button></Item>
            <Item label="Focus"><Button kind="primary" className="gallery-focus">Create task</Button></Item>
            <Item label="Busy"><Button kind="primary" aria-busy="true">Creating…</Button></Item>
            <Item label="Disabled"><Button kind="primary" disabled>Create task</Button></Item>
          </div>
          <div className="gallery-row">
            <Item label="Secondary"><Button>Edit</Button></Item>
            <Item label="Hover"><Button className="gallery-hover">Edit</Button></Item>
            <Item label="Quiet"><Button kind="quiet">Cancel</Button></Item>
            <Item label="Small, 28"><Button size="small">New task</Button></Item>
            <Item label="Medium, 34"><Button>New task</Button></Item>
            <Item label="Large, 44"><Button kind="primary" size="large">Continue <span aria-hidden="true">→</span></Button></Item>
          </div>
          <div className="gallery-row">
            <Item label="Danger, secondary"><Button className="button-danger">Remove</Button></Item>
            <Item label="Danger, quiet"><Button kind="quiet" className="button-danger">Remove sign-in</Button></Item>
            <Item label="Quiet, disabled"><Button kind="quiet" disabled>Change</Button></Item>
            <Item label="Icon"><Button kind="icon" aria-label="Close"><CloseIcon /></Button><Button kind="icon" aria-label="Copy"><CopyIcon /></Button></Item>
            <Item label="Large, secondary"><Button size="large">Use a cluster</Button></Item>
          </div>
        </Section>
        <Section title="Fields and choices" note="Errors appear under the field and say what to do.">
          <div className="gallery-row">
            <Item label="Input"><input className="field gallery-field" aria-label="Input" placeholder="Filter by name" /></Item>
            <Item label="Focus"><input className="field gallery-field gallery-focus" aria-label="Focus" placeholder="Filter by name" /></Item>
            <Item label="Filled"><input className="field gallery-field" aria-label="Filled" defaultValue="web-reader" /></Item>
          </div>
          <div className="gallery-row">
            <Item label="Error"><span className="gallery-stack"><input className="field gallery-field" aria-label="Error" aria-invalid="true" aria-describedby="gallery-error" defaultValue="my agent" /><span className="field-error" id="gallery-error">Use lowercase letters, numbers and dashes.</span></span></Item>
            <Item label="Help"><span className="gallery-stack"><input className="field gallery-field" aria-label="Help" aria-describedby="gallery-hint" /><span className="field-hint" id="gallery-hint">A short name you will recognise.</span></span></Item>
            <Item label="Disabled"><input className="field gallery-field" aria-label="Disabled" disabled defaultValue="web-reader" /></Item>
          </div>
          <div className="gallery-row">
            <Item label="Select"><select className="field gallery-field gallery-mono" aria-label="Select" defaultValue="web-reader"><option>web-reader</option><option>triage</option></select></Item>
            <Item label="Select, disabled"><select className="field gallery-field" aria-label="Select, disabled" disabled><option>web-reader</option></select></Item>
            <Item label="Textarea"><textarea className="field gallery-field" aria-label="Textarea" rows={2} defaultValue="Read the page and answer." /></Item>
          </div>
          <div className="gallery-row">
            <Item label="Segmented"><div className="view-switch" role="group" aria-label="View"><button type="button" aria-pressed="true">Agent</button><button type="button" aria-pressed="false">Try a task</button><button type="button" aria-pressed="false">File</button></div></Item>
            <Item label="Checkbox"><label className="gallery-check"><input type="checkbox" defaultChecked />Pull requests</label></Item>
            <Item label="Radio"><label className="gallery-check"><input type="radio" name="gallery-radio" defaultChecked />This computer</label></Item>
          </div>
        </Section>
        <Section title="Messages and small parts">
          <div className="gallery-row">
            <Item label="Alert, failed"><div className="alert alert-failed" role="alert"><p className="alert-title">Can’t reach team-dev. Check your network or VPN.</p><p>Your workspace and agent files are kept.</p></div></Item>
            <Item label="Alert, needs you"><div className="alert alert-needs-you"><p className="alert-title">Connect the AI to try your agent.</p><p>Nothing runs until it is connected.</p></div></Item>
          </div>
          <div className="gallery-row">
            <Item label="Empty state"><div className="empty-state">No tasks run yet. The result shows here.</div></Item>
            <Item label="Disclosure"><Disclosure label="Details" hint="Optional"><p>Folded content.</p></Disclosure></Item>
            <Item label="Disclosure, open"><Disclosure label="More settings" defaultOpen><p>Folded content.</p></Disclosure></Item>
          </div>
          <div className="gallery-row">
            <Item label="Code"><code className="code-chip">control/orka</code></Item>
            <Item label="Key"><span><kbd className="key">⌘K</kbd> <kbd className="key">Esc</kbd></span></Item>
            <Item label="Chip"><Chip>In use</Chip></Item>
            <Item label="Label"><span className="section-label">This computer</span></Item>
            <Item label="Generated value"><GeneratedValue label="Agent hash" value="sha256:7c17abcdefa370" /></Item>
          </div>
        </Section>
      </div>
      <div className="gallery-column">
        <Section title="Status" note="Six statuses. A pill holds one word; the icon carries the same status in rows.">
          <table className="gallery-table">
            <thead><tr><th scope="col" className="section-label">Pill</th><th scope="col" className="section-label">Icon</th><th scope="col" className="section-label">Words</th><th scope="col" className="section-label">Means</th></tr></thead>
            <tbody>{statuses.map(({ state, words, means }) => <tr key={state}>
              <td><StatusPill state={state} /></td>
              <td><StatusIcon state={state} /></td>
              <td><span className="gallery-words">{words.map(word => <StatusPill key={word} state={state}>{word}</StatusPill>)}</span></td>
              <td>{means}</td>
            </tr>)}</tbody>
          </table>
        </Section>
        <Section title="Cards" note="A plain card has a 1px line. The card that holds the next action has a 1.5px ink edge; a card being edited has 2px.">
          <div className="gallery-cards">
            <div className="card card-strong gallery-confirm">
              <div className="gallery-confirm-head"><span className="section-label">Proposed task</span><StatusPill state="not-started">Not created</StatusPill></div>
              <dl className="gallery-rows"><dt>Agent</dt><dd><code className="code-chip">web-reader</code></dd><dt>Runs on</dt><dd>This computer</dd></dl>
              <div className="gallery-actions"><Button kind="primary">Create task</Button><Button>Edit</Button><Button kind="quiet">Just answer here</Button></div>
            </div>
            <div className="card"><LongCard title="Local tests" status={<StatusPill state="failed" />} action={<Button size="small">Run again</Button>} summary="One of two tests failed on this computer."><Details hint="Task and hashes"><GeneratedValue label="Task" value="bridge-task-1234567890" /></Details></LongCard></div>
            <div className="card card-editing"><h3>Editing</h3><p className="muted">A card in edit mode.</p></div>
          </div>
        </Section>
        <Section title="Type">
          <p className="gallery-hero">Hero</p>
          <p className="gallery-display">Display</p>
          <p className="gallery-page">Page title</p>
          <p className="gallery-object">web-reader</p>
          <p className="gallery-section-title">Section</p>
          <h1>Title, 21</h1>
          <h3>Card title, 15</h3>
          <p className="reading">Reading, 15. The prompt, a test’s question, the agent’s answer.</p>
          <p>Body, 13. Everything else, including help text and status sentences.</p>
          <p className="gallery-small">Small, 11.5. Times and counts.</p>
        </Section>
      </div>
    </div>
  </main>;
}
