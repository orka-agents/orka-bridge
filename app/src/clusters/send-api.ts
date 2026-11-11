import { invoke, isTauri } from '@tauri-apps/api/core';
import { isSourceId, type SourceId } from '../designer/agent-api';
import { isRunSnapshot, type RunSnapshot } from '../designer/run-api';

export type SendView = Pick<RunSnapshot, 'acknowledged' | 'pending' | 'report'> & {
  id: string; targetId: string; providerId: string; targetName: string | null;
  source: SourceId | null; model: string | null; agentName: string | null;
  state: 'running' | 'complete' | 'unknown';
  stage: 'capture' | 'access' | 'preflight' | 'submit' | 'storage' | 'observe' | 'stopped' | 'done';
  error: string | null; storageError: boolean;
};
const uuid = (v: unknown) => typeof v === 'string' && /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i.test(v);
const text = (v: unknown) => v === null || typeof v === 'string' && v.length > 0 && v.length <= 4096;
export const sendErrors = {
  capture: 'Your saved agent or check files changed, or could not be read. Check this destination again before sending.',
  access: 'Cluster access could not be confirmed. Choose your cluster again in Clusters.',
  preflight: 'The installation, tools or Provider could not be confirmed. Check your cluster and try again.',
  storage: 'Nothing was sent because Bridge could not save a summary. Check local storage before trying again.',
  submit: 'Sending was not confirmed. Named resources may still exist. Nothing was retried.',
  observe: 'The result could not be confirmed. The cluster may still be running checks.',
  stopped: 'Stopped watching. The cluster may still be running checks.',
} as const;
function valid(v: unknown): v is SendView {
  if (typeof v !== 'object' || v === null || Array.isArray(v)) return false;
  const row = v as Record<string, unknown>;
  return Object.keys(row).sort().join(',') === 'acknowledged,agentName,error,id,model,pending,providerId,report,source,stage,state,storageError,targetId,targetName'
    && typeof row.storageError === 'boolean'
    && uuid(row.id) && uuid(row.targetId) && uuid(row.providerId) && text(row.targetName)
    && (row.source === null || isSourceId(row.source)) && text(row.model) && text(row.agentName)
    && ['running', 'complete', 'unknown'].includes(row.state as string)
    && ['capture', 'access', 'preflight', 'submit', 'storage', 'observe', 'stopped', 'done'].includes(row.stage as string)
    && (row.error === null || typeof row.error === 'string' && Object.hasOwn(sendErrors, row.error))
    && isRunSnapshot({ state: row.state, canRun: false, report: row.report, acknowledged: row.acknowledged, pending: row.pending, error: null });
}
export type SendCheck = {
  source: SourceId | null; targetId: string; providerId: string; model: string | null;
  rows: { label: string; status: 'allowed' | 'blocked' | 'cannotCheck' | 'unverified'; message: string }[];
  canSend: boolean;
};
function isCheck(v: unknown): v is SendCheck {
  if (typeof v !== 'object' || v === null || Array.isArray(v)) return false;
  const row = v as Record<string, unknown>;
  return Object.keys(row).sort().join(',') === 'canSend,model,providerId,rows,source,targetId'
    && uuid(row.targetId) && uuid(row.providerId) && text(row.model)
    && (row.source === null || isSourceId(row.source))
    && Array.isArray(row.rows) && row.rows.length > 0 && row.rows.length <= 1024
    && row.rows.every(r => r && typeof r === 'object' && Object.keys(r).sort().join(',') === 'label,message,status'
      && typeof r.label === 'string' && text(r.label) && typeof r.message === 'string' && text(r.message)
      && ['allowed', 'blocked', 'cannotCheck', 'unverified'].includes(r.status))
    && typeof row.canSend === 'boolean'
    && (!row.canSend || row.source !== null && row.model !== null && row.rows.every(r => r.status === 'allowed' || r.status === 'unverified'));
}
export async function sendPreflight(targetId: string, providerId: string): Promise<SendCheck> {
  if (isTauri()) {
    try {
      const value: unknown = await invoke('send_preflight', { targetId, providerId });
      if (isCheck(value) && value.targetId === targetId && value.providerId === providerId) return value;
    } catch { /* Do not forward native diagnostics. */ }
  }
  throw Error('Destination checks could not be read. Check your cluster and try again.');
}
export type AttemptSummary = {
  id: string; source: SourceId; targetDisplay: string;
  targetUids: { serviceNamespace: string; serviceUid: string; deploymentNamespace: string; deploymentUid: string; watchedNamespace: string };
  provider: string; plannedAgentName: string; agentUid: string | null;
  verdict: 'running' | 'passed' | 'failed' | 'unknown'; time: number;
};
function isSummary(value: unknown): value is AttemptSummary {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
  const row = value as Record<string, unknown>;
  const identity = row.targetUids;
  return Object.keys(row).sort().join(',') === 'agentUid,id,plannedAgentName,provider,source,targetDisplay,targetUids,time,verdict'
    && uuid(row.id) && isSourceId(row.source)
    && [row.targetDisplay, row.provider, row.plannedAgentName].every(v => v !== null && text(v))
    && text(row.agentUid)
    && ['running', 'passed', 'failed', 'unknown'].includes(row.verdict as string)
    && typeof row.time === 'number' && Number.isSafeInteger(row.time) && row.time >= 0 && row.time <= 253402300799
    && typeof identity === 'object' && identity !== null && !Array.isArray(identity)
    && Object.keys(identity).sort().join(',') === 'deploymentNamespace,deploymentUid,serviceNamespace,serviceUid,watchedNamespace'
    && Object.values(identity).every(v => v !== null && text(v));
}
export async function sendHistory(): Promise<AttemptSummary[]> {
  if (isTauri()) {
    try {
      const value: unknown = await invoke('send_history');
      if (Array.isArray(value) && value.length <= 1024 && value.every(isSummary)) return value;
    } catch { /* Stored bytes and diagnostics are not display data. */ }
  }
  throw Error('Saved Send summaries could not be read. They are unavailable, not passing results.');
}
export async function sendCall(command: 'send_start' | 'send_status' | 'send_stop', args: Record<string, unknown> = {}): Promise<SendView | null> {
  if (!isTauri()) throw Error('Send is available only in the desktop app.');
  try { const value: unknown = await invoke(command, args); if (valid(value) || command === 'send_status' && value === null) return value as SendView | null; } catch { /* No native diagnostics or credentials reach the screen. */ }
  throw Error('Send information could not be read. Nothing was retried. Reopen this view to check the result.');
}
