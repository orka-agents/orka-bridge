import { invoke, isTauri } from '@tauri-apps/api/core';
import { type ChatCommand, type ChatResult, type ChatSnapshot, chatErrors, isChatSnapshot } from '../designer/chat-api';

// The key of the Navigator's one live conversation (navigator.rs CONVERSATION). Never a saved agent file.
export const NAVIGATOR_ID = '6e617669-6761-4074-9f2e-6e6176696761';
export type ThinksWith = { kind: 'copilot' | 'openai' | 'anthropic'; model: string };
export type NavigatorSnapshot = { chat: ChatSnapshot; thinksWith: ThinksWith | null };
export type NavigatorResult = ChatResult & { thinksWith?: ThinksWith | null };

// The same native errors as Try a task, worded for a conversation with the Navigator.
export const navigatorErrors: Record<keyof typeof chatErrors, string> = {
  invalidMessage: 'Write a shorter message before trying again.',
  busy: 'The workspace is busy with a test or task. Send again when it finishes.',
  unsupportedTool: 'Bridge could not prepare the Navigator. Try again.',
  source: 'Bridge could not prepare the Navigator. Try again.',
  model: 'Connect the AI in Clusters, then try again.',
  connection: 'The workspace could not be reached. Nothing was created or changed.',
  identity: 'The answer could not be verified. Start a new conversation.',
  response: 'The answer could not be read. Reconnect to check again.',
  contextUnavailable: 'This conversation could not be confirmed. Start a new conversation.',
  workspaceChanged: 'The workspace changed. Start a new conversation.',
  interrupted: 'The answer was interrupted. Nothing was created or changed.',
};

const commands: Record<ChatCommand, string> = {
  agent_chat_status: 'navigator_status', agent_chat_send: 'navigator_send', agent_chat_reconnect: 'navigator_reconnect',
  agent_chat_stop: 'navigator_stop', agent_chat_new: 'navigator_new',
};
const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const exactKeys = (value: Record<string, unknown>, keys: string[]) => Object.keys(value).sort().join() === [...keys].sort().join();
// Exact: a reply with an unknown key, or another conversation's id, is not a Navigator reply.
export function isNavigatorSnapshot(value: unknown): value is NavigatorSnapshot {
  if (!object(value) || !exactKeys(value, ['chat', 'thinksWith'])) return false;
  const { chat, thinksWith } = value;
  return object(chat) && exactKeys(chat, ['documentId', 'state', 'busy', 'canSend', 'versions', 'error']) && isChatSnapshot(chat)
    && chat.documentId === NAVIGATOR_ID
    && chat.versions.every(version => exactKeys(version, ['turns']) && version.turns.every(turn => exactKeys(turn, ['message', 'answer', 'phase', 'tools', 'contextRetained', 'terminationUnconfirmed', 'stopUnconfirmed', 'startedAt', 'finishedAt'])
      && turn.tools.every(tool => exactKeys(tool, ['name', 'callId', 'phase']))))
    && (thinksWith === null || object(thinksWith) && exactKeys(thinksWith, ['kind', 'model'])
      && ['copilot', 'openai', 'anthropic'].includes(thinksWith.kind as string) && typeof thinksWith.model === 'string' && thinksWith.model.length > 0 && thinksWith.model.length <= 256);
}

// Speaks the Try a task command set so useTurn can reconcile the Navigator's turns the same way.
export async function navigatorCall(command: ChatCommand, _documentId: string, message?: string): Promise<NavigatorResult> {
  if (!isTauri()) return { ok: false, message: 'The Navigator is available only in the native app.' };
  try {
    const value = await invoke<unknown>(commands[command], command === 'agent_chat_send' ? { message } : {});
    if (isNavigatorSnapshot(value)) return { ok: true, snapshot: value.chat, thinksWith: value.thinksWith };
  } catch (error) {
    if (typeof error === 'string' && Object.hasOwn(navigatorErrors, error)) return { ok: false, message: navigatorErrors[error as keyof typeof chatErrors] };
  }
  return { ok: false, uncertain: true, message: 'The Navigator’s reply could not be confirmed. Reconnect to check again without sending it again.' };
}
