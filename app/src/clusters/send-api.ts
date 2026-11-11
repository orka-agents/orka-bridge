import { invoke, isTauri } from '@tauri-apps/api/core';
import { isDocumentRef, isDocumentId, SAMPLE_DOCUMENT_ID, isSourceId, type DocumentRef, type SourceId } from '../designer/agent-api';
import { isRunSnapshot, type RunSnapshot } from '../designer/run-api';

export type SendView = Pick<RunSnapshot, 'acknowledged' | 'pending' | 'report'> & {
  documentId: string; id: string; targetId: string; providerId: string; targetName: string | null;
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
  return Object.keys(row).sort().join(',') === 'acknowledged,agentName,documentId,error,id,model,pending,providerId,report,source,stage,state,targetId,targetName'
    && isDocumentId(row.documentId) && uuid(row.id) && uuid(row.targetId) && uuid(row.providerId) && text(row.targetName)
    && (row.source === null || isSourceId(row.source)) && text(row.model) && text(row.agentName)
    && ['running', 'complete', 'unknown'].includes(row.state as string)
    && ['capture', 'access', 'preflight', 'submit', 'observe', 'stopped', 'done'].includes(row.stage as string)
    && (row.error === null || typeof row.error === 'string' && Object.hasOwn(sendErrors, row.error))
    && isRunSnapshot({ state: row.state, canRun: false, report: row.report, acknowledged: row.acknowledged, pending: row.pending, error: null });
}
export type SendCheck = {
  documentId: string; source: SourceId | null; targetId: string; providerId: string; model: string | null;
  rows: { label: string; status: 'allowed' | 'blocked' | 'cannotCheck' | 'unverified'; message: string }[];
  canSend: boolean;
};
function isCheck(v: unknown): v is SendCheck {
  if (typeof v !== 'object' || v === null || Array.isArray(v)) return false;
  const row = v as Record<string, unknown>;
  return Object.keys(row).sort().join(',') === 'canSend,documentId,model,providerId,rows,source,targetId'
    && isDocumentId(row.documentId) && uuid(row.targetId) && uuid(row.providerId) && text(row.model)
    && (row.source === null || isSourceId(row.source))
    && Array.isArray(row.rows) && row.rows.length > 0 && row.rows.length <= 1024
    && row.rows.every(r => r && typeof r === 'object' && Object.keys(r).sort().join(',') === 'label,message,status'
      && typeof r.label === 'string' && text(r.label) && typeof r.message === 'string' && text(r.message)
      && ['allowed', 'blocked', 'cannotCheck', 'unverified'].includes(r.status))
    && typeof row.canSend === 'boolean'
    && (!row.canSend || row.source !== null && row.model !== null && row.rows.every(r => r.status === 'allowed' || r.status === 'unverified'));
}
export async function sendPreflight(targetId: string, providerId: string, documentId = SAMPLE_DOCUMENT_ID): Promise<SendCheck> {
  if (isTauri()) {
    try {
      const value: unknown = await invoke('send_preflight', { documentId, targetId, providerId });
      if (isCheck(value) && value.documentId === documentId && value.targetId === targetId && value.providerId === providerId) return value;
    } catch { /* Do not forward native diagnostics. */ }
  }
  throw Error('Destination checks could not be read. Check your cluster and try again.');
}
export type SendStatus = { documentId: string; view: SendView | null; busy: DocumentRef | null };
export async function sendCall(command: 'send_start' | 'send_status' | 'send_stop', args: Record<string, unknown> = {}): Promise<SendStatus> {
  if (!isTauri()) throw Error('Send is available only in the desktop app.');
  const documentId = args.documentId ?? SAMPLE_DOCUMENT_ID;
  try {
    const value: unknown = await invoke(command, { ...args, documentId });
    if (typeof value === 'object' && value !== null && Object.keys(value).sort().join(',') === 'busy,documentId,view'
      && 'documentId' in value && value.documentId === documentId && isDocumentId(value.documentId)
      && 'view' in value && (value.view === null || valid(value.view) && value.view.documentId === documentId)
      && 'busy' in value && (value.busy === null || isDocumentRef(value.busy))) return value as SendStatus;
  } catch { /* No native diagnostics or credentials reach the screen. */ }
  throw Error('Send information could not be read. Nothing was retried. Reopen this view to check the result.');
}
