import digest from './navigator/digest.md?raw';

// The Navigator's reply is data. A card is drawn only from one fenced `navigator-card` block whose JSON has exactly
// the fields of a known kind; anything else stays plain text. A card can only offer what its native command allows,
// and every card waits for a click (navigator/instructions.md describes the same shapes to the model).
export type TaskProposal = { kind: 'task'; agent: string; task: string };
export type AgentProposal = { kind: 'agent'; name: string; description: string; instructions: string; tools: string[] };
export type ExplainAction = { label: string; ask: string } | { label: string; open: 'Overview' | 'Designer' | 'Clusters' };
export type ExplainCard = { kind: 'explain'; title: string; rows: { term: string; text: string }[]; actions: ExplainAction[] };
export type Card = TaskProposal | AgentProposal | ExplainCard;
export type Reply = { prose: string; card?: Card; sources: string[] };

// Tools an agent card may add: common built-ins a local agent can run. Approval and reply tools are never offered.
export const agentCardTools = ['web_fetch', 'web_search', 'file_read', 'file_write', 'code_exec'] as const;
export const pages = ['Overview', 'Designer', 'Clusters'] as const;
const agentName = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/;
// Every source path the digest names; an answer may cite only these.
export const digestSources: ReadonlySet<string> = new Set([...digest.matchAll(/^Source: (\S+)$/gm)].map(match => match[1]));

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const keys = (value: Record<string, unknown>, required: string[], optional: string[] = []) =>
  required.every(key => Object.hasOwn(value, key)) && Object.keys(value).every(key => required.includes(key) || optional.includes(key));
const text = (value: unknown, max: number, lines = true): value is string =>
  typeof value === 'string' && value.trim().length > 0 && value.length <= max && (lines || !/[\r\n]/.test(value));

export function validCard(value: unknown): value is Card {
  if (!object(value)) return false;
  switch (value.kind) {
    case 'task':
      return keys(value, ['kind', 'agent', 'task']) && typeof value.agent === 'string' && agentName.test(value.agent) && text(value.task, 4000);
    case 'agent':
      return keys(value, ['kind', 'name', 'description', 'instructions', 'tools']) && typeof value.name === 'string' && agentName.test(value.name)
        && text(value.description, 200, false) && text(value.instructions, 8000)
        && Array.isArray(value.tools) && value.tools.length <= agentCardTools.length && new Set(value.tools).size === value.tools.length
        && value.tools.every(tool => agentCardTools.includes(tool as typeof agentCardTools[number]));
    case 'explain':
      return keys(value, ['kind', 'title', 'rows'], ['actions']) && text(value.title, 60, false)
        && Array.isArray(value.rows) && value.rows.length >= 2 && value.rows.length <= 6
        && value.rows.every(row => object(row) && keys(row, ['term', 'text']) && text(row.term, 40, false) && text(row.text, 300, false))
        && (value.actions === undefined || Array.isArray(value.actions) && value.actions.length <= 2 && value.actions.every(action => object(action) && text(action.label, 40, false)
          && (keys(action, ['label', 'ask']) && text(action.ask, 500) || keys(action, ['label', 'open']) && pages.includes(action.open as typeof pages[number]))));
    default:
      return false;
  }
}

const block = /^```navigator-card[ \t]*\r?\n([\s\S]*?)\r?\n```[ \t]*$/gm;
// Splits a reply into prose, at most one validated card and the digest pages it cites.
export function parseReply(answer: string): Reply {
  let prose = answer.replace(/\r\n/g, '\n');
  let card: Card | undefined;
  const blocks = [...prose.matchAll(block)];
  // One proposal per reply: two blocks, or one that fails validation, are shown as written.
  if (blocks.length === 1) {
    let value: unknown;
    try { value = JSON.parse(blocks[0][1]); } catch { value = undefined; }
    if (validCard(value)) {
      card = value.kind === 'explain' ? { ...value, actions: value.actions ?? [] } : value;
      prose = (prose.slice(0, blocks[0].index) + prose.slice(blocks[0].index! + blocks[0][0].length)).trim();
    }
  }
  let sources: string[] = [];
  const lines = prose.trimEnd().split('\n');
  const last = /^Sources?:\s*(.+)$/.exec(lines.at(-1)?.trim() ?? '');
  if (last) {
    const cited = last[1].split(',').map(source => source.trim().replace(/^`|`$/g, '')).filter(Boolean);
    if (cited.length && cited.every(source => digestSources.has(source))) {
      sources = [...new Set(cited)];
      prose = lines.slice(0, -1).join('\n').trimEnd();
    }
  }
  return { prose: prose.trim(), card, sources };
}

// `orka:concepts/memory.md#safety-model` reads as "Orka docs · concepts/memory.md".
export function sourceLabel(source: string) {
  const [origin, rest] = source.split(':', 2);
  return { site: origin === 'orka' ? 'Orka docs' : 'Bridge docs', page: (rest ?? '').split('#')[0] };
}
