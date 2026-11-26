import type { ReactNode } from 'react';

// The Navigator's prose as text: paragraphs, bulleted or numbered lists, `code` and **bold**. Nothing is parsed as
// HTML, links are not followed, and anything else shows exactly as written.
function inline(text: string): ReactNode[] {
  const parts: ReactNode[] = [];
  const pattern = /`([^`\n]+)`|\*\*([^*\n]+)\*\*/g;
  let at = 0;
  for (const match of text.matchAll(pattern)) {
    if (match.index > at) parts.push(text.slice(at, match.index));
    parts.push(match[1] !== undefined ? <code key={match.index}>{match[1]}</code> : <strong key={match.index}>{match[2]}</strong>);
    at = match.index + match[0].length;
  }
  if (at < text.length) parts.push(text.slice(at));
  return parts;
}

export default function Prose({ text }: { text: string }) {
  const blocks = text.split(/\n{2,}/).map(block => block.trim()).filter(Boolean);
  return <div className="nav-prose">{blocks.map((block, index) => {
    const lines = block.split('\n');
    const bullets = lines.every(line => /^\s*[-*] /.test(line));
    const numbers = lines.every(line => /^\s*\d+[.)] /.test(line));
    if (bullets || numbers) {
      const items = lines.map((line, item) => <li key={item}>{inline(line.replace(/^\s*(?:[-*]|\d+[.)]) /, ''))}</li>);
      return numbers ? <ol key={index}>{items}</ol> : <ul key={index}>{items}</ul>;
    }
    return <p key={index}>{lines.flatMap((line, item) => item ? [<br key={`b${item}`} />, ...inline(line)] : inline(line))}</p>;
  })}</div>;
}
