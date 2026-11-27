import { invoke, isTauri } from '@tauri-apps/api/core';

// Start a fresh workspace: the native side stops the workspace, sets it aside, removes Bridge's
// saved AI sign-in or key and starts first install. The renderer sends nothing with it.
const refusals = ['stopUnconfirmed', 'busy', 'inUse', 'layout', 'updateHistory', 'unavailable'] as const;
export type FreshRefusal = (typeof refusals)[number];
export type FreshAi = 'removed' | 'absent' | 'kept';
export type FreshStatus =
  | { state: 'replacing' }
  | { state: 'replaced'; transaction: string; ai: FreshAi; reclaimable: string[]; cleaning: boolean }
  | { state: 'refused'; reason: FreshRefusal };
const uuid = /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i;
const keys = (value: object) => Object.keys(value).sort().join(',');

export function isFreshStatus(value: unknown): value is FreshStatus {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const v = value as Record<string, unknown>;
  switch (v.state) {
    case 'replacing': return keys(v) === 'state';
    case 'refused': return keys(v) === 'reason,state' && refusals.some(reason => reason === v.reason);
    case 'replaced': return keys(v) === 'ai,cleaning,reclaimable,state,transaction'
      && typeof v.transaction === 'string' && uuid.test(v.transaction)
      && (v.ai === 'removed' || v.ai === 'absent' || v.ai === 'kept') && typeof v.cleaning === 'boolean'
      && Array.isArray(v.reclaimable) && v.reclaimable.length <= 9
      && v.reclaimable.every(path => typeof path === 'string' && path.length > 0 && path.length <= 4096);
    default: return false;
  }
}

export async function startFresh(): Promise<FreshStatus> {
  if (!isTauri()) return { state: 'refused', reason: 'unavailable' };
  try {
    const value = await invoke<unknown>('workspace_fresh_start');
    if (isFreshStatus(value)) return value;
  } catch { /* An unreadable reply is reported as unavailable. */ }
  return { state: 'refused', reason: 'unavailable' };
}

// Every refusal leaves the workspace on this computer as it was.
export const refusalCopy: Record<FreshRefusal, string> = {
  stopUnconfirmed: "Bridge couldn't confirm the workspace stopped, so it didn't replace it. Quit Bridge and reopen it, then try again.",
  busy: 'Another workspace action is still running. Try again when it finishes.',
  inUse: "Something on this computer still has the workspace's files open, so Bridge didn't replace it. Quit Bridge and reopen it, then try again.",
  layout: "Bridge found workspace files it doesn't recognise, so it left them as they are. Copy the details for whoever helps you with Bridge.",
  updateHistory: 'This workspace came from a workspace update, and Bridge keeps the earlier workspace so it can be restored. Bridge can\'t start a fresh workspace over that yet. Copy the details for whoever helps you with Bridge.',
  unavailable: "Bridge couldn't start a fresh workspace. Copy the details for whoever helps you with Bridge.",
};

// For Copy details for help: finite states only, never a path.
export function freshDetails(status?: FreshStatus): string[] {
  if (!status) return [];
  if (status.state === 'replacing') return ['Fresh workspace: replacing'];
  if (status.state === 'refused') return [`Fresh workspace: refused (${status.reason})`];
  return [`Fresh workspace: replaced (${status.transaction})`, `Fresh workspace AI sign-in: ${status.ai}`,
    `Fresh workspace cleanup: ${status.cleaning ? 'running' : status.reclaimable.length ? `${status.reclaimable.length} folder${status.reclaimable.length > 1 ? 's' : ''} left to delete` : 'done'}`];
}
