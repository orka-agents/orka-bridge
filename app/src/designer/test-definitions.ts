import { parseDocument } from 'yaml';
import patterns from './test-title-patterns.json';
export type TestDefinition = { id: string; title?: string; question: string };
const credentials = patterns.map(pattern => new RegExp(pattern.replace(/^\(\?i\)/, ''), pattern.startsWith('(?i)') ? 'iu' : 'u'));
export function validTestTitle(value: unknown): value is string {
  return typeof value === 'string' && !!value.trim() && [...value].length <= 160
    && !/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/u.test(value) && !credentials.some(pattern => pattern.test(value));
}
const text = (value: unknown): value is string => typeof value === 'string' && !!value.trim() && !value.includes('\0');
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
function yaml(source: string): unknown {
  const doc = parseDocument(source);
  if (doc.errors.length) throw Error('Unsupported tests');
  return doc.toJS({ maxAliasCount: 100 });
}
// Descriptive only: native Run still validates/captures the saved pair before execution.
// Keep fence recognition identical to engine/run_source.rs, including other_fence.
export function testDefinitions(source: string): TestDefinition[] | undefined {
  if (new TextEncoder().encode(source).length > 1048576) return undefined;
  try {
    const lines = source.split('\n').map(line => line.replace(/\r$/, ''));
    if (lines.shift() !== '---') return undefined;
    const end = lines.indexOf('---');
    if (end < 0) return undefined;
    const header = yaml(lines.splice(0, end + 1).slice(0, -1).join('\n'));
    if (!object(header) || header.schemaVersion !== 1 || Object.keys(header).length !== 1) return undefined;
    const cases: TestDefinition[] = [];
    const ids = new Set<string>();
    let block: string[] | undefined, otherFence = false;
    for (const line of lines) {
      if (line === '```yaml agent-test' && !otherFence) {
        if (block) return undefined;
        block = [];
      } else if (line === '```' && block) {
        const value = yaml(block.join('\n')); block = undefined;
        if (!object(value) || Object.keys(value).some(key => !['id', 'title', 'input', 'assertions'].includes(key))
          || !text(value.id) || ids.has(value.id) || !text(value.input)
          || 'title' in value && !validTestTitle(value.title)
          || !Array.isArray(value.assertions) || !value.assertions.length || value.assertions.length > 64) return undefined;
        const assertions = new Set<string>();
        for (const a of value.assertions) {
          if (!object(a) || !text(a.id) || assertions.has(a.id)
            || !['contains', 'toolCalled'].includes(a.type as string)
            || Object.keys(a).sort().join(',') !== (a.type === 'contains' ? 'id,type,value' : 'id,tool,type')
            || !text(a.type === 'contains' ? a.value : a.tool)) return undefined;
          assertions.add(a.id);
        }
        ids.add(value.id);
        cases.push({ id: value.id, question: value.input, ...('title' in value ? { title: value.title as string } : {}) });
        if (cases.length > 32) return undefined;
      } else if (block) block.push(line);
      else if (line.startsWith('```')) {
        if (line.includes('agent-test')) return undefined;
        otherFence = !otherFence;
      }
    }
    return !block && cases.length ? cases : undefined;
  } catch { return undefined; }
}
