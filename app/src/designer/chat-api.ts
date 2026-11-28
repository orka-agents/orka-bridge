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
  interrupted: 'Stopped before the task was created. Nothing ran.',
  permission: 'Your access to this cluster doesn’t allow this task. Ask the cluster’s administrator for access.',
  sessionsUnsupported: 'This cluster’s Orka is too old for Try a task. It needs Orka v0.4.0 or later.',
  clusterUnavailable: 'Bridge couldn’t reach the selected cluster. Reconnect it in Clusters, then try again.',
  workspaceOutdated: 'The workspace on this computer is older than this Bridge and can’t run this yet. Nothing was sent to it.',
  workspaceUnavailable: 'The workspace on this computer stopped answering. Quit Bridge and open it again to restart the workspace. Your agents and files are kept.',
  placeChanged: 'This conversation was started with another AI connection. Start a new conversation to continue.',
  accessLost: 'Access to the selected cluster needs reconnecting. Reconnect it in Clusters to see this task’s result.',
} as const;
// The Orka permission a cluster refused (chat_cluster.rs ChatPermission), as the person's administrator grants it.
export const chatPermissions = {
  createAgents: 'create agents', getAgents: 'get agents', deleteAgents: 'delete agents',
  createTasks: 'create tasks', getTasks: 'get tasks', deleteTasks: 'delete tasks',
  getSessions: 'get sessions', deleteSessions: 'delete sessions', getProviders: 'get providers',
} as const;
export type ChatPermission = keyof typeof chatPermissions;
export function deniedMessage(permission: ChatPermission) {
  return `Your access to this cluster doesn’t allow “${chatPermissions[permission]}” in its namespace. Ask the cluster’s administrator to add it.`;
}
// The native error for a snapshot, naming the refused permission when the cluster said which.
export function snapshotErrorMessage(snapshot: ChatSnapshot, errors: Record<keyof typeof chatErrors, string> = chatErrors) {
  if (!snapshot.error) return undefined;
  return snapshot.error === 'permission' && snapshot.denied ? deniedMessage(snapshot.denied) : errors[snapshot.error];
}
// `finishedBeforeStop`: Stop arrived after this turn's Task had already finished, so nothing was stopped.
export type ChatTurn = { message: string; answer: string | null; phase: string; tools: { name: string; callId: string; phase: string }[]; contextRetained: boolean; terminationUnconfirmed: boolean; stopUnconfirmed: boolean; startedAt: number; finishedAt: number | null; finishedBeforeStop?: boolean };
export type ChatSnapshot = {
  documentId: string; state: typeof chatStates[number]; busy: boolean; canSend: boolean; versions: { turns: ChatTurn[] }[]; error: keyof typeof chatErrors | null;
  // With error `permission` on a cluster: what it refused. `keptOnCluster` latches once a cluster refused to delete a replaced conversation.
  denied: ChatPermission | null; keptOnCluster: boolean;
  // A cluster conversation started on another AI connection: shown, never continued, until New.
  readOnly: boolean;
};
// A connected cluster and AI connection, by the opaque ids native already confirmed. Absent means this computer.
export type ChatPlace = { targetId: string; providerId: string };
export type ChatCommand = 'agent_chat_status' | 'agent_chat_send' | 'agent_chat_reconnect' | 'agent_chat_stop' | 'agent_chat_new';
// Unix milliseconds from the native clock.
const millis = (value: unknown) => Number.isSafeInteger(value) && (value as number) >= 0;
const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
export function isChatSnapshot(value: unknown): value is ChatSnapshot {
  return object(value) && isDocumentId(value.documentId) && chatStates.includes(value.state as ChatSnapshot['state'])
    && typeof value.busy === 'boolean' && typeof value.canSend === 'boolean'
    && (value.error === null || typeof value.error === 'string' && Object.hasOwn(chatErrors, value.error))
    && (value.denied === null || typeof value.denied === 'string' && Object.hasOwn(chatPermissions, value.denied)) && typeof value.keptOnCluster === 'boolean' && typeof value.readOnly === 'boolean'
    && Array.isArray(value.versions) && value.versions.every(version => object(version) && Array.isArray(version.turns) && version.turns.every(turn =>
      object(turn) && typeof turn.message === 'string' && (turn.answer === null || typeof turn.answer === 'string') && typeof turn.phase === 'string'
      && ['contextRetained', 'terminationUnconfirmed', 'stopUnconfirmed'].every(key => typeof turn[key] === 'boolean')
      && (turn.finishedBeforeStop === undefined || typeof turn.finishedBeforeStop === 'boolean')
      && millis(turn.startedAt) && (turn.finishedAt === null || millis(turn.finishedAt))
      && Array.isArray(turn.tools) && turn.tools.every(tool => object(tool) && typeof tool.name === 'string' && typeof tool.callId === 'string' && typeof tool.phase === 'string')));
}
export type ChatResult = { ok: true; snapshot: ChatSnapshot } | { ok: false; message: string; uncertain?: true };
export type ChatCall = (command: ChatCommand, documentId: string, message?: string, place?: ChatPlace) => Promise<ChatResult>;
// Every command names the selected place, so each place answers with its own conversation.
export async function chatCall(command: ChatCommand, documentId: string, message?: string, place?: ChatPlace): Promise<ChatResult> {
  if (!isTauri()) return { ok: false, message: 'Try a task is available only in the native app.' };
  try {
    const value = await invoke<unknown>(command, { documentId, ...(command === 'agent_chat_send' ? { message } : {}), ...place });
    if (isChatSnapshot(value) && value.documentId === documentId) return { ok: true, snapshot: value };
  } catch (error) {
    if (typeof error === 'string' && Object.hasOwn(chatErrors, error)) return { ok: false, message: chatErrors[error as keyof typeof chatErrors] };
  }
  return { ok: false, uncertain: true, message: 'Task information could not be confirmed. Reconnect to check again without running it again.' };
}
