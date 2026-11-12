import { useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';

export default function FileLocation({ path, documentId }: { path?: string; documentId: string }) {
  const [feedback, setFeedback] = useState<{ path: string; text: string }>();
  if (!path) return <p className="muted">The file path is unavailable.</p>;
  const filePath = path;
  async function copy() {
    try { await navigator.clipboard.writeText(filePath); setFeedback({ path: filePath, text: 'Path copied' }); }
    catch { setFeedback({ path: filePath, text: 'Could not copy the path' }); }
  }
  async function reveal() {
    try {
      if (!isTauri()) throw Error('Desktop only');
      await invoke('agent_reveal', { documentId });
      setFeedback({ path: filePath, text: 'Folder opened' });
    } catch { setFeedback({ path: filePath, text: 'Could not show the folder. Copy the path to open it yourself.' }); }
  }
  return <div className="file-location"><code className="agent-path">{path}</code><div className="agent-tool-row">
    <button type="button" className="part-button" aria-label="Copy file path" onClick={() => { void copy(); }}>Copy path</button>
    <button type="button" className="part-button" onClick={() => { void reveal(); }}>Show in folder</button>
    <span className="muted" role="status">{feedback?.path === path ? feedback.text : ''}</span>
  </div></div>;
}
