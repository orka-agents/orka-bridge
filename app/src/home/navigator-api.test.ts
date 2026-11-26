import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { SAMPLE_DOCUMENT_ID } from '../designer/agent-api';
import { NAVIGATOR_ID, isNavigatorSnapshot, navigatorCall } from './navigator-api';

const chat = { documentId: NAVIGATOR_ID, state: 'ready', busy: false, canSend: true, versions: [{ turns: [{ message: 'Hi', answer: 'Hello', phase: 'Succeeded', tools: [{ name: 'x', callId: 'c', phase: 'ToolCallCompleted' }], contextRetained: true, terminationUnconfirmed: false, stopUnconfirmed: false, startedAt: 1, finishedAt: 2 }] }], error: null };
const reply = { chat, thinksWith: { kind: 'copilot', model: 'gpt-5.6-terra' } };
beforeEach(() => { vi.stubGlobal('isTauri', true); });
afterEach(() => { clearMocks(); vi.unstubAllGlobals(); });

test('a Navigator reply is exact: no unknown key at any level and only the Navigator conversation', () => {
  expect(isNavigatorSnapshot(reply)).toBe(true);
  expect(isNavigatorSnapshot({ ...reply, thinksWith: null })).toBe(true);
  for (const bad of [
    { ...reply, extra: 1 }, { chat }, { ...reply, chat: { ...chat, documentId: SAMPLE_DOCUMENT_ID } }, { ...reply, chat: { ...chat, extra: 1 } },
    { ...reply, chat: { ...chat, versions: [{ turns: [{ ...chat.versions[0].turns[0], extra: 1 }] }] } },
    { ...reply, chat: { ...chat, versions: [{ turns: [], extra: 1 }] } },
    { ...reply, thinksWith: { kind: 'copilot', model: 'm', provider: 'p' } }, { ...reply, thinksWith: { kind: 'other', model: 'm' } },
  ]) expect(isNavigatorSnapshot(bad)).toBe(false);
});

test('commands map to the Navigator set, and native errors read as Navigator words', async () => {
  const seen: [string, unknown][] = [];
  mockIPC((command, args) => {
    seen.push([command, args]);
    if (command === 'navigator_send') throw 'busy';
    if (command === 'navigator_stop') return { ...reply, chat: { ...chat, documentId: SAMPLE_DOCUMENT_ID } };
    return reply;
  });
  expect(await navigatorCall('agent_chat_status', 'ignored')).toMatchObject({ ok: true, thinksWith: reply.thinksWith });
  expect(await navigatorCall('agent_chat_send', 'ignored', 'Hello')).toEqual({ ok: false, message: 'The workspace is busy with a test or task. Send again when it finishes.' });
  // An unexpected reply is never taken as a refusal: the outcome is uncertain.
  expect(await navigatorCall('agent_chat_stop', 'ignored')).toMatchObject({ ok: false, uncertain: true });
  expect(seen).toEqual([['navigator_status', {}], ['navigator_send', { message: 'Hello' }], ['navigator_stop', {}]]);
});
