import { invoke, isTauri } from '@tauri-apps/api/core';

const states = ['none', 'eligible', 'sourceSelected', 'sourceRetained', 'fresh', 'incomplete', 'consumed', 'restoring', 'restored', 'conflict'] as const;
const credentials = ['absent', 'eligible', 'attention', 'transferred', 'removed'] as const;
const errors = ['network', 'noSpace', 'inUse', 'stopUnconfirmed', 'integrity', 'unsupported', 'unknown'] as const;
export type UpgradeStatus = {
  state: typeof states[number]; transaction: string | null; createdAt: number | null;
  sourceReleases: string[]; targetRelease: string | null;
  retainedAllocatedBytes: number | null; additionalBytes: number | null;
  credential: typeof credentials[number]; busy: boolean; error: typeof errors[number] | null;
  repeated: boolean; canReplace: boolean; canRestore: boolean;
};
export type UpgradeAction = 'status' | 'replace' | 'retry' | 'restore' | 'transfer' | 'remove' | 'cancel';
const uuid = (value: unknown): value is string => typeof value === 'string'
  && /^[\da-f]{8}(-[\da-f]{4}){3}-[\da-f]{12}$/i.test(value) && value !== '00000000-0000-0000-0000-000000000000';
const integer = (value: unknown) => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
function decode(value: unknown): UpgradeStatus | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || Object.keys(value).sort().join(',') !== 'additionalBytes,busy,canReplace,canRestore,createdAt,credential,error,repeated,retainedAllocatedBytes,sourceReleases,state,targetRelease,transaction') return;
  const s = value as UpgradeStatus;
  if (!states.includes(s.state) || !credentials.includes(s.credential) || (s.error !== null && !errors.includes(s.error))
    || (s.transaction !== null && !uuid(s.transaction)) || (s.createdAt !== null && !integer(s.createdAt))
    || !Array.isArray(s.sourceReleases) || s.sourceReleases.length > 32 || !s.sourceReleases.every(v => typeof v === 'string' && v.length > 0 && v.length <= 128)
    || (s.targetRelease !== null && (typeof s.targetRelease !== 'string' || !s.targetRelease.length || s.targetRelease.length > 128))
    || (s.retainedAllocatedBytes !== null && !integer(s.retainedAllocatedBytes)) || (s.additionalBytes !== null && !integer(s.additionalBytes))
    || typeof s.busy !== 'boolean' || typeof s.repeated !== 'boolean' || typeof s.canReplace !== 'boolean' || typeof s.canRestore !== 'boolean'
    || (!['none', 'eligible', 'conflict'].includes(s.state) && !s.transaction)
    || (s.canReplace && !['eligible', 'sourceSelected', 'sourceRetained', 'incomplete', 'restored'].includes(s.state))
    || (s.canRestore && (!s.transaction || ['none', 'eligible', 'restored', 'conflict'].includes(s.state)))
    || ((s.canReplace || s.canRestore) && (!s.sourceReleases.length || !s.targetRelease))) return;
  return s;
}
export async function upgradeCall(action: UpgradeAction, transaction?: string): Promise<{ ok: true; status: UpgradeStatus } | { ok: false; message: string }> {
  const failure = { ok: false as const, message: 'Workspace update information is unavailable. Check again before changing your workspace.' };
  if (!isTauri() || (['retry', 'restore', 'transfer', 'remove'].includes(action) && !uuid(transaction))) return failure;
  try {
    const value = await (transaction ? invoke<unknown>(`workspace_upgrade_${action}`, { transaction }) : invoke<unknown>(`workspace_upgrade_${action}`));
    const status = decode(value);
    return status ? { ok: true, status } : failure;
  } catch { return failure; }
}
