import { Button } from '../components/Button';
import { RelativeTime } from '../components/RelativeTime';
import type { Group } from './model';

export function PlaceNotes({ groups, actions }: { groups: Group[]; actions: { onCheck: () => void; onClusters: () => void } }) {
  return <div className="overview-notes">{groups.map(group => {
    const notes = group.notes;
    return notes.length ? <section key={group.key} aria-label={`${group.title} status`}>
      {notes.map(note => <div key={note.text} className="overview-note alert" role="status">
        <span className={group.key === 'local' ? 'alert-title' : 'alert-title overview-mono'}>{group.title}:</span><p>{note.text}{note.seen !== undefined && <> Showing what Bridge saw <RelativeTime value={note.seen} inline />.</>}</p>
        {note.check && <Button size="small" onClick={actions.onCheck}>Check again</Button>}
        {/* Beside Check again, Open Clusters is the quiet second way on. */}
        {note.clusters && <Button kind={note.check ? 'quiet' : 'secondary'} size="small" onClick={actions.onClusters}>Open Clusters</Button>}
      </div>)}
    </section> : null;
  })}</div>;
}
