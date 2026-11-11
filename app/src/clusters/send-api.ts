import { invoke, isTauri } from '@tauri-apps/api/core';
import { isSourceId, type SourceId } from '../designer/agent-api';
import { isRunSnapshot, type RunSnapshot } from '../designer/run-api';

export type SendView = Pick<RunSnapshot, 'acknowledged' | 'pending' | 'report'> & {
  id: string; targetId: string; providerId: string; targetName: string | null;
  source: SourceId | null; model: string | null; agentName: string | null;
  state: 'running' | 'complete' | 'unknown';
  stage: 'capture' | 'access' | 'preflight' | 'submit' | 'observe' | 'stopped' | 'done';
  error: string | null;
};
const uuid = (v: unknown) => typeof v === 'string' && /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i.test(v);
const text = (v: unknown) => v === null || typeof v === 'string' && v.length > 0 && v.length <= 4096;
export const sendErrors = {
  capture: 'Your saved agent or check files changed, or could not be read. Check this destination again before sending.',
  access: 'Cluster access could not be confirmed. Choose your cluster again in Clusters.',
  preflight: 'The installation, tools or Provider could not be confirmed. Check your cluster and try again.',
  submit: 'Sending was not confirmed. Named resources may still exist. Nothing was retried.',
  observe: 'The result could not be confirmed. The cluster may still be running checks.',
  stopped: 'Stopped watching. The cluster may still be running checks.',
} as const;
function valid(v: unknown): v is SendView {
  if (typeof v !== 'object' || v === null || Array.isArray(v)) return false;
  const row = v as Record<string, unknown>;
  return Object.keys(row).sort().join(',') === 'acknowledged,agentName,error,id,model,pending,providerId,report,source,stage,state,targetId,targetName'
    && uuid(row.id) && uuid(row.targetId) && uuid(row.providerId) && text(row.targetName)
    && (row.source === null || isSourceId(row.source)) && text(row.model) && text(row.agentName)
    && ['running', 'complete', 'unknown'].includes(row.state as string)
    && ['capture', 'access', 'preflight', 'submit', 'observe', 'stopped', 'done'].includes(row.stage as string)
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
export async function sendCall(command: 'send_start' | 'send_status' | 'send_stop', args: Record<string, unknown> = {}): Promise<SendView | null> {
  if (!isTauri()) throw Error('Send is available only in the desktop app.');
  try { const value: unknown = await invoke(command, args); if (valid(value) || command === 'send_status' && value === null) return value as SendView | null; } catch { /* No native diagnostics or credentials reach the screen. */ }
  throw Error('Send information could not be read. Nothing was retried. Reopen this view to check the result.');
}
