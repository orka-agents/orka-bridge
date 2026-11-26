import { invoke, isTauri } from '@tauri-apps/api/core';
import { unsupportedToolMessage } from './built-in-tools';
import { isDocumentId } from './agent-api';
export const chatStates = ['ready', 'preparing', 'working', 'waitingForInput', 'waitingForSession', 'observationLost', 'requestingStop', 'stopRequested', 'stopUnconfirmed', 'contextUnavailable'] as const;
export const chatErrors = {
  invalidMessage: 'Write a shorter task before trying again.',
  busy: 'The workspace is busy. Wait for the current test or task, then try again.',
  unsupportedTool: unsupportedToolMessage,
  source: 'Check your saved agent file before trying again.',
  model: 'Connect the AI in Clusters and check the model in your agent file.',
  connection: 'The connection was lost. Reconnect to check what happened; do not run the task again.',
  identity: 'The task could not be verified. You can run a new task.',
  response: 'The task could not be read. Reconnect to check what happened.',
  contextUnavailable: 'The task could not be confirmed. You can run a new task.',
  workspaceChanged: 'The workspace changed. Run the task again if you still need it.',
  interrupted: 'The task was interrupted. It may still be running.',
} as const;
export type ChatTurn = { message: string; answer: string | null; phase: string; tools: { name: string; callId: string; phase: string }[]; contextRetained: boolean; terminationUnconfirmed: boolean; stopUnconfirmed: boolean; startedAt: number; finishedAt: number | null };
export type ChatSnapshot = { documentId: string; state: typeof chatStates[number]; busy: boolean; canSend: boolean; versions: { turns: ChatTurn[] }[]; error: keyof typeof chatErrors | null };
export type ChatCommand = 'agent_chat_status' | 'agent_chat_send' | 'agent_chat_reconnect' | 'agent_chat_stop' | 'agent_chat_new';
// Unix milliseconds from the native clock.
const millis = (value: unknown) => Number.isSafeInteger(value) && (value as number) >= 0;
const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
export function isChatSnapshot(value: unknown): value is ChatSnapshot {
  return object(value) && isDocumentId(value.documentId) && chatStates.includes(value.state as ChatSnapshot['state'])
    && typeof value.busy === 'boolean' && typeof value.canSend === 'boolean'
    && (value.error === null || typeof value.error === 'string' && Object.hasOwn(chatErrors, value.error))
    && Array.isArray(value.versions) && value.versions.every(version => object(version) && Array.isArray(version.turns) && version.turns.every(turn =>
      object(turn) && typeof turn.message === 'string' && (turn.answer === null || typeof turn.answer === 'string') && typeof turn.phase === 'string'
      && ['contextRetained', 'terminationUnconfirmed', 'stopUnconfirmed'].every(key => typeof turn[key] === 'boolean')
      && millis(turn.startedAt) && (turn.finishedAt === null || millis(turn.finishedAt))
      && Array.isArray(turn.tools) && turn.tools.every(tool => object(tool) && typeof tool.name === 'string' && typeof tool.callId === 'string' && typeof tool.phase === 'string')));
}
export type ChatResult = { ok: true; snapshot: ChatSnapshot } | { ok: false; message: string; uncertain?: true };
export type ChatCall = (command: ChatCommand, documentId: string, message?: string) => Promise<ChatResult>;
export async function chatCall(command: ChatCommand, documentId: string, message?: string): Promise<ChatResult> {
  if (!isTauri()) return { ok: false, message: 'Try a task is available only in the native app.' };
  try {
    const value = await invoke<unknown>(command, { documentId, ...(command === 'agent_chat_send' ? { message } : {}) });
    if (isChatSnapshot(value) && value.documentId === documentId) return { ok: true, snapshot: value };
  } catch (error) {
    if (typeof error === 'string' && Object.hasOwn(chatErrors, error)) return { ok: false, message: chatErrors[error as keyof typeof chatErrors] };
  }
  return { ok: false, uncertain: true, message: 'Task information could not be confirmed. Reconnect to check again without running it again.' };
}
