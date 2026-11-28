import { invoke, isTauri } from '@tauri-apps/api/core';
import { formatNeededGB, formatRequirementGB } from './requirements';

// Start a fresh workspace: the native side stops the workspace, sets it aside, removes Bridge's
// saved AI sign-in or key and starts first install. The renderer sends nothing with it.
const refusals = ['stopUnconfirmed', 'busy', 'inUse', 'layout', 'updateHistory', 'offline', 'noSpace', 'unavailable'] as const;
export type FreshRefusal = (typeof refusals)[number];
const outcomes = ['removed', 'absent', 'kept', 'unconfirmed'] as const;
export type FreshAi = (typeof outcomes)[number];
export type FreshStatus =
  | { state: 'replacing' }
  | { state: 'replaced'; transaction: string; ai: FreshAi; reclaimable: string[]; cleaning: boolean }
  // Nothing was moved or deleted.
  | { state: 'refused'; reason: FreshRefusal; restarted: boolean; neededBytes?: number; reclaimable?: string[] }
  // Set aside, not finished: Bridge finishes it on the next try or when it opens.
  | { state: 'interrupted'; transaction: string }
  // Folders Bridge didn't expect, left as they are; the workspace waits until they're moved.
  | { state: 'conflict'; found: string[] };
// Whether the action can start, and what first install will need.
export type FreshImage = { kind: 'cached' } | { kind: 'download'; bytes?: number } | { kind: 'unknown' };
export type FreshPlan = { blocked?: FreshRefusal; image: FreshImage };
const uuid = /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i;
const keys = (value: object) => Object.keys(value).sort().join(',');
const paths = (value: unknown, max: number) => Array.isArray(value) && value.length <= max
  && value.every(path => typeof path === 'string' && path.length > 0 && path.length <= 4096);
const bytes = (value: unknown) => typeof value === 'number' && Number.isSafeInteger(value) && value > 0;
const refusal = (value: unknown): value is FreshRefusal => refusals.some(reason => reason === value);

export function isFreshStatus(value: unknown): value is FreshStatus {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const v = value as Record<string, unknown>;
  switch (v.state) {
    case 'replacing': return keys(v) === 'state';
    case 'refused': return refusal(v.reason) && typeof v.restarted === 'boolean'
      && Object.keys(v).every(key => ['reason', 'restarted', 'state'].includes(key)
        || key === 'neededBytes' && v.reason === 'noSpace' && bytes(v.neededBytes)
        || key === 'reclaimable' && paths(v.reclaimable, 9) && (v.reclaimable as unknown[]).length > 0);
    case 'replaced': return keys(v) === 'ai,cleaning,reclaimable,state,transaction'
      && typeof v.transaction === 'string' && uuid.test(v.transaction)
      && outcomes.some(ai => ai === v.ai) && typeof v.cleaning === 'boolean' && paths(v.reclaimable, 9);
    case 'interrupted': return keys(v) === 'state,transaction' && typeof v.transaction === 'string' && uuid.test(v.transaction);
    case 'conflict': return keys(v) === 'found,state' && paths(v.found, 2) && (v.found as unknown[]).length > 0;
    default: return false;
  }
}
export function isFreshPlan(value: unknown): value is FreshPlan {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const v = value as Record<string, unknown>;
  if (keys(v) !== (v.blocked === undefined ? 'image' : 'blocked,image') || (v.blocked !== undefined && !refusal(v.blocked))) return false;
  const image = v.image;
  if (!image || typeof image !== 'object' || Array.isArray(image)) return false;
  const i = image as Record<string, unknown>;
  if (i.kind === 'download') return keys(i) === (i.bytes === undefined ? 'kind' : 'bytes,kind') && (i.bytes === undefined || bytes(i.bytes));
  return (i.kind === 'cached' || i.kind === 'unknown') && keys(i) === 'kind';
}

const unavailable: FreshStatus = { state: 'refused', reason: 'unavailable', restarted: false };
export async function startFresh(): Promise<FreshStatus> {
  if (!isTauri()) return unavailable;
  try {
    const value = await invoke<unknown>('workspace_fresh_start');
    if (isFreshStatus(value)) return value;
  } catch { /* An unreadable reply is reported as unavailable. */ }
  return unavailable;
}

// The confirmation's line about the download, from what native found in the cache.
export function downloadLine(plan?: FreshPlan): string {
  const image = plan?.image;
  if (image?.kind === 'cached') return 'Bridge reuses the download already on this computer.';
  if (image?.kind === 'download') return image.bytes ? `Bridge downloads the workspace again: about ${formatRequirementGB(image.bytes)}.` : 'Bridge downloads the workspace again.';
  return "If the workspace's download isn't on this computer any more, Bridge downloads it again.";
}

export const updateHistoryCopy = "This workspace came from a workspace update, and Bridge keeps the earlier workspace so it can be restored. Bridge can't start a fresh workspace over that yet. Copy the details for whoever helps you with Bridge.";
// Every refusal leaves the workspace on this computer as it was.
export function refusalCopy(status: Extract<FreshStatus, { state: 'refused' }>): string {
  switch (status.reason) {
    case 'stopUnconfirmed': return "Bridge couldn't confirm the workspace stopped, so it didn't replace it. Quit Bridge and reopen it, then try again.";
    case 'busy': return 'Another workspace action is still running. Try again when it finishes.';
    case 'inUse': return "Something on this computer still has the workspace's files open, so Bridge didn't replace it. Quit Bridge and reopen it, then try again.";
    case 'layout': return "Bridge found workspace files it doesn't recognise, so it left them as they are. Copy the details for whoever helps you with Bridge.";
    case 'updateHistory': return updateHistoryCopy;
    case 'offline': return "Bridge needs to download the workspace again and couldn't reach the download. Check your internet connection, then try again.";
    case 'noSpace': return `Bridge needs to download the workspace again and this computer doesn't have enough free space${status.neededBytes ? `: it needs about ${formatNeededGB(status.neededBytes)} free` : ''}. Free up space, then try again.`;
    case 'unavailable': return "Bridge couldn't start a fresh workspace. Copy the details for whoever helps you with Bridge.";
  }
}

// For Copy details for help: finite states only, never a path.
export function freshDetails(status?: FreshStatus, plan?: FreshPlan): string[] {
  const lines: string[] = [];
  if (status?.state === 'replacing') lines.push('Fresh workspace: replacing');
  if (status?.state === 'refused') lines.push(`Fresh workspace: refused (${status.reason}${status.restarted ? ', workspace started again' : ''})`);
  if (status?.state === 'interrupted') lines.push(`Fresh workspace: interrupted (${status.transaction})`);
  if (status?.state === 'conflict') lines.push(`Fresh workspace: unexpected folders (${status.found.length})`);
  if (status?.state === 'replaced') lines.push(`Fresh workspace: replaced (${status.transaction})`, `Fresh workspace AI sign-in: ${status.ai}`,
    `Fresh workspace cleanup: ${status.cleaning ? 'running' : status.reclaimable.length ? `${status.reclaimable.length} folder${status.reclaimable.length > 1 ? 's' : ''} left to delete` : 'done'}`);
  if (status?.state === 'refused' && status.reclaimable?.length) lines.push(`Fresh workspace cleanup: ${status.reclaimable.length} folder${status.reclaimable.length > 1 ? 's' : ''} left to delete`);
  if (plan?.blocked) lines.push(`Fresh workspace available: no (${plan.blocked})`);
  return lines;
}
